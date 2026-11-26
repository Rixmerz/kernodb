//! Tests of the trigram index on the emulated engine and a simulated disk,
//! like those of the keyword index.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;

use std::collections::BTreeMap;

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

/// The index of `unit_type` as a search reads it: the units of each trigram, in id order.
fn index_of(db: &UnitDb<E>, unit_type: &str) -> BTreeMap<Vec<u8>, Vec<UnitId>> {
    let snap = db.raw_snapshot().unwrap();
    runs::dump_units::<_, Trigrams>(&snap, &type_prefix(unit_type))
        .into_iter()
        .map(|(gram, units)| (gram, units.into_iter().map(|u| u.0).collect()))
        .collect()
}

/// The units the index of `sym` holds, each once, in id order.
fn indexed_units(db: &UnitDb<E>) -> Vec<UnitId> {
    let mut out: Vec<UnitId> = index_of(db, "sym").into_values().flatten().collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The manifest of the index of `sym`.
fn manifest(db: &UnitDb<E>) -> Manifest {
    let snap = db.raw_snapshot().unwrap();
    runs::read_manifest::<Trigrams>(&|k| snap.get(k), &type_prefix("sym")).unwrap()
}

/// The number of `unit` in the index of `sym`.
fn number_of(db: &UnitDb<E>, unit: UnitId) -> u64 {
    let raw = db.raw_get(&runs::doc_key(&type_prefix("sym"), unit)).unwrap().expect("a document record");
    runs::decode_doc(&raw, "TRIGRAM").unwrap().0
}

/// The raw writes that leave the postings of `sym` as `edit` makes them, in one run whose
/// manifest agrees with its pages.
fn forged(
    db: &UnitDb<E>,
    edit: impl FnOnce(&mut BTreeMap<Vec<u8>, Vec<runs::Posting>>),
) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    runs::forge::<_, Trigrams>(&db.raw_snapshot().unwrap(), &type_prefix("sym"), edit)
}

/// The run pages of the index of `sym`, as `(key, value)`.
fn pages(db: &UnitDb<E>) -> Vec<(Vec<u8>, Vec<u8>)> {
    let from = runs::kind_key(&type_prefix("sym"), runs::KIND_RUN);
    entries(db).into_iter().filter(|(k, _)| k.starts_with(&from)).collect()
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
        assert_eq!(mark(&db), Some(crate::codec::MIN_LAYER_VERSION), "a database starts at the cut version");
        db.set_substring("sym", true, ChangeMeta::default()).unwrap();
        db.set_substring("sym", false, ChangeMeta::default()).unwrap();
        assert_eq!(mark(&db), Some(crate::codec::MIN_LAYER_VERSION));
        db.set_substring("sym", true, ChangeMeta::default()).unwrap();
        db.close().unwrap();
    }
    let db = open_plain(&disk);
    assert_eq!(found(&db, b"plain").len(), 1);
    assert_clean(&db);
}

// ------------------------------------------------------------ long lists

/// 300 units hold "common", 100 of them also "middle", and one holds "rare": the list of
/// "com" is more pages of a merged run than a search reads at first.
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
    let m = manifest(&db);
    assert!(m.runs.len() > 1 && m.runs.len() < 3 * runs::MERGE_FANOUT, "the runs were merged in tiers: {m:?}");
    assert!(all.len() > PAGE * runs::PAGE_TARGET && middle.len() > PROBE_MAX);
    // One long list, read past its first pages: a three-byte needle and a two-byte one.
    assert_eq!(found(&db, b"com"), all);
    assert_eq!(found(&db, b"co"), all);
    assert_eq!(found(&db, b"c"), all);
    // Few candidates ("rar" is one unit) against the long lists of "common": probes.
    assert_eq!(found(&db, b"common rare"), [rare]);
    assert_eq!(found(&db, b"mon r"), [rare]);
    // A probe that finds the page of a candidate without the candidate in it.
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
fn a_change_set_writes_one_run_and_dead_numbers_leave_at_a_merge() {
    let disk = SimVfs::new(20);
    let db = open(&disk);
    let new =
        |p: String| NewUnit { unit_type: "sym".into(), schema_version: 1, metadata: vec![], payload: p.into_bytes() };
    // One change set of 300 units: one run, its lists cut in pages.
    let ids: Vec<UnitId> = change(&db, |cs| {
        (0..300).map(|i| cs.create(new(if i == 50 { "other".into() } else { format!("word {i}") })).unwrap()).collect()
    });
    let m = manifest(&db);
    assert_eq!((m.next, m.live, m.runs.len()), (300, 300, 1));
    assert_eq!((m.runs[0].min, m.runs[0].max, m.runs[0].level, m.runs[0].dead), (0, 299, 0, 0));
    assert_eq!(m.runs[0].pages as usize, pages(&db).len());
    assert!(m.runs[0].pages > 3, "the run is several pages");
    assert_eq!(index_of(&db, "sym")[&b"wor"[..]].len(), 299);
    assert_clean(&db);
    // An update gives the unit a new number in a new run; the old number dies in its run.
    update(&db, ids[50], b"word 50");
    assert_eq!(found(&db, b"word"), ids);
    let m = manifest(&db);
    assert_eq!((m.next, m.runs.len(), m.runs[0].dead, m.runs[1].dead), (301, 2, 1, 0));
    assert_eq!(number_of(&db, ids[50]), 300);
    assert!(found(&db, b"other").is_empty(), "the postings of a dead number are not read");
    assert_clean(&db);
    // Deletes only mark numbers dead: no run is written, and the postings stay until a merge.
    change(&db, |cs| {
        for id in &ids[..100] {
            cs.delete(*id, None).unwrap();
        }
    });
    let m = manifest(&db);
    assert_eq!((m.runs.len(), m.runs[0].dead, m.runs[1].dead), (2, 100, 1));
    assert_eq!(found(&db, b"word"), ids[100..]);
    assert_clean(&db);
    // Enough change sets to fill a tier: the runs are merged and the dead numbers dropped.
    let more: Vec<UnitId> =
        (0..runs::MERGE_FANOUT).map(|i| store(&db, format!("word more {i}").as_bytes())).collect();
    let m = manifest(&db);
    assert!(m.runs.iter().all(|r| r.dead == 0), "{m:?}");
    assert_eq!(m.runs.iter().map(|r| r.postings).sum::<u64>() as usize, index_of(&db, "sym").values().map(Vec::len).sum());
    assert_eq!(found(&db, b"word"), [&ids[100..], &more[..]].concat());
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
    let m = manifest(&db);
    assert_eq!((m.runs.len(), m.runs[0].min), (1, 0), "the index was written again from nothing");
    assert_clean(&db);
    // The last unit leaving takes the index back to no record at all.
    change(&db, |cs| {
        for id in indexed_units(&db) {
            cs.delete(id, None).unwrap();
        }
    });
    assert!(entries(&db).is_empty(), "an index that holds nothing has no record");
    assert_clean(&db);
    let again = store(&db, b"word again");
    assert_eq!((found(&db, b"again"), number_of(&db, again)), (vec![again], 0));
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
    let mut units: Vec<(UnitId, Vec<u8>)> = (0..120)
        .map(|i| {
            let payload = text(3 + i % 23);
            (store(&db, &payload), payload)
        })
        .collect();
    // Some are rewritten and some deleted, so runs hold dead numbers.
    for i in (0..120).step_by(7) {
        let payload = text(5 + (i as u64) % 11);
        update(&db, units[i].0, &payload);
        units[i].1 = payload;
    }
    for i in (3..120).step_by(11).rev() {
        let (id, _) = units.remove(i);
        change(&db, |cs| cs.delete(id, None).unwrap());
    }
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
    let (num, other_num) = (number_of(&db, id), number_of(&db, other));

    // The unit is gone from one list it is in, in pages the manifest agrees with.
    db.raw_commit(forged(&db, |lists| lists.get_mut(&b"wor"[..]).unwrap().retain(|p| p.0 != num))).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // The unit is in no list.
    db.raw_commit(forged(&db, |lists| lists.values_mut().for_each(|l| l.retain(|p| p.0 != num)))).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // A trigram the payload does not call for.
    db.raw_commit(forged(&db, |lists| lists.entry(b"zzz".to_vec()).or_default().push((num, 0, 0)))).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // A posting of a number that was never given.
    db.raw_commit(forged(&db, |lists| lists.entry(b"qqq".to_vec()).or_default().push((7, 0, 0)))).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // A number whose slot names a unit whose type has no index, and one that does not exist.
    let page = runs::num_key(&type_prefix("sym"), 0);
    for unit in [note, UnitId([0xEE; 16])] {
        let slots = vec![(true, unit), (true, other)];
        db.raw_commit(vec![(page.clone(), Some(runs::encode_num(&slots)))]).unwrap();
        assert_eq!(rules(&db), ["UNIT-52"]);
        restore(&db);
    }

    // The slots of two units swapped: each has the trigrams of the other.
    db.raw_commit(vec![(page.clone(), Some(runs::encode_num(&[(true, other), (true, id)])))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);
    // Swapped with their document records too: only the payloads tell.
    let swapped = vec![
        (page.clone(), Some(runs::encode_num(&[(true, other), (true, id)]))),
        (runs::doc_key(&type_prefix("sym"), id), Some(runs::encode_doc(other_num, 0))),
        (runs::doc_key(&type_prefix("sym"), other), Some(runs::encode_doc(num, 0))),
    ];
    db.raw_commit(swapped).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // The whole index under another type.
    let moved: Vec<_> = before
        .iter()
        .map(|(k, v)| ([&type_prefix("doc")[..], &k[type_prefix("sym").len()..]].concat(), Some(v.clone())))
        .collect();
    db.raw_commit(moved).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);
}

/// Each record of the index lost alone, and a record that was not written by the index.
#[test]
fn verify_and_a_search_see_a_lost_record() {
    let disk = SimVfs::new(21);
    let db = open(&disk);
    let ids: Vec<UnitId> = (0..40).map(|i| store(&db, format!("hello number {i}").as_bytes())).collect();
    update(&db, ids[0], b"hello again");
    assert_clean(&db);
    let m = manifest(&db);
    assert!(m.runs.len() > 1 && m.runs.iter().any(|r| r.pages > 1), "{m:?}");
    let corrupt = |r: &Result<Vec<UnitId>, UnitError>| matches!(r, Err(UnitError::Storage(kerno_core::Error::Corruption(_))));
    let all = entries(&db);
    let mut refused = 0;
    // Any run page: verify sees the run disagree with the manifest, and a search of a list
    // the page held, or that begins after it, is refused instead of answering short.
    for (k, v) in pages(&db) {
        db.raw_commit(vec![(k.clone(), None)]).unwrap();
        assert_eq!(rules(&db), ["UNIT-52"], "page {k:?}");
        for needle in [&b"hel"[..], b"llo", b"num", b"ber", b"hello number", b"o n", b"aga"] {
            let r = db.search_substring(needle, None, None);
            refused += usize::from(corrupt(&r));
            assert!(corrupt(&r) || r.unwrap() == found_in(&ids, needle), "page {k:?}, needle {needle:?}");
        }
        db.raw_commit(vec![(k, Some(v))]).unwrap();
        assert_clean(&db);
    }
    assert!(refused > 0, "a search that needs a lost page is refused");
    // The manifest, a document record, a number page.
    let prefix = type_prefix("sym");
    let lost = [runs::manifest_key(&prefix), runs::doc_key(&prefix, ids[5]), runs::num_key(&prefix, 0)];
    for k in lost {
        let v = all.iter().find(|(key, _)| *key == k).map(|(_, v)| v.clone()).expect("the record exists");
        db.raw_commit(vec![(k.clone(), None)]).unwrap();
        assert_eq!(rules(&db), ["UNIT-52"], "key {k:?}");
        db.raw_commit(vec![(k, Some(v))]).unwrap();
        assert_clean(&db);
    }
    // A page of a run the manifest does not list, and a page added to a run it lists.
    let (k, v) = pages(&db).into_iter().next().unwrap();
    let at = prefix.len() + 1;
    let mut stray = k.clone();
    stray[at..at + 8].copy_from_slice(&9_999u64.to_be_bytes());
    db.raw_commit(vec![(stray.clone(), Some(v.clone()))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    db.raw_commit(vec![(stray, None)]).unwrap();
    assert_clean(&db);
    // A page added after the last page of a run the manifest lists, naming that page.
    let last = pages(&db).into_iter().rfind(|(key, _)| key[..at + 8] == k[..at + 8]).unwrap();
    let mut builder = runs::RunBuilder::new::<Trigrams>(0).after(&last.0[at + 8..]);
    builder.begin(&[0xFF, 0xFF, 0xFF]);
    builder.push((0, 0, 0));
    let (suffix, value) = builder.finish().unwrap().pages.pop().unwrap();
    let extra = [&k[..at + 8], &suffix[..]].concat();
    db.raw_commit(vec![(extra, Some(value))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
}

/// The units among `ids` (the first rewritten to "hello again") that hold `needle`.
fn found_in(ids: &[UnitId], needle: &[u8]) -> Vec<UnitId> {
    let text = |i: usize| if i == 0 { "hello again".to_string() } else { format!("hello number {i}") };
    ids.iter()
        .enumerate()
        .filter(|(i, _)| text(*i).as_bytes().windows(needle.len()).any(|w| w == needle))
        .map(|(_, id)| *id)
        .collect()
}

#[test]
fn verify_reports_an_entry_that_does_not_decode() {
    let disk = SimVfs::new(17);
    let db = open(&disk);
    store(&db, b"hello");
    let (k, v) = pages(&db).into_iter().next().unwrap();
    let partial = [&v[..], &[0u8; 5]].concat();
    // No envelope, version 0, flags, no header, a newer version, trailing bytes, a page cut
    // inside an item, and numbers that do not ascend.
    let mut descending = v.clone();
    let n = descending.len();
    descending[n - 1] = 0;
    for bad in [vec![], vec![0, 0], vec![1, 1], vec![1, 0], vec![2, 0], partial, v[..v.len() - 1].to_vec()] {
        db.raw_commit(vec![(k.clone(), Some(bad.clone()))]).unwrap();
        assert!(!db.verify_units().unwrap().is_clean(), "value {bad:?}");
    }
    db.raw_commit(vec![(k.clone(), Some(v.clone()))]).unwrap();
    assert_clean(&db);
    // A key one byte short, one with an empty type, and one of no kind of record.
    let unknown = [&type_prefix("sym")[..], &[7u8][..]].concat();
    for bad_key in [k[..k.len() - 1].to_vec(), [&[TAG_TRIGRAM, 0][..], &[b'a'; 20]].concat(), unknown] {
        db.raw_commit(vec![(bad_key.clone(), Some(v.clone()))]).unwrap();
        assert!(!db.verify_units().unwrap().is_clean(), "key {bad_key:?}");
        db.raw_commit(vec![(bad_key, None)]).unwrap();
        assert_clean(&db);
    }
    assert!(decode_prefix(&[TAG_TRIGRAM]).is_err());
    assert!(decode_prefix(&[&[TAG_TRIGRAM, 1, 0xFF][..], &[b'a'; 4]].concat()).is_err(), "a type that is not UTF-8");
    assert_eq!(decode_prefix(&[&[TAG_TRIGRAM, 1, b's'][..], &[0]].concat()).unwrap(), ("s", 3));
    // The other records of the index: each damaged alone.
    let prefix = type_prefix("sym");
    for key in [runs::manifest_key(&prefix), runs::doc_key(&prefix, found(&db, b"hello")[0]), runs::num_key(&prefix, 0)]
    {
        let good = db.raw_get(&key).unwrap().unwrap();
        for bad in [vec![1, 0], vec![2, 0], [&good[..], &[0u8]].concat(), good[..good.len() - 1].to_vec()] {
            db.raw_commit(vec![(key.clone(), Some(bad.clone()))]).unwrap();
            assert!(!db.verify_units().unwrap().is_clean(), "key {key:?} value {bad:?}");
        }
        db.raw_commit(vec![(key, Some(good))]).unwrap();
        assert_clean(&db);
    }
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
    db.raw_commit(vec![(runs::num_key(&type_prefix("sym"), 0), Some(runs::encode_num(&[(true, ghost)])))]).unwrap();
    assert!(matches!(
        db.search_substring(b"hello", None, None),
        Err(UnitError::Storage(kerno_core::Error::Corruption(m))) if m.contains("no UNIT_HEAD")
    ));
    // A posting of a number that has no slot.
    db.raw_commit(vec![(runs::num_key(&type_prefix("sym"), 0), None)]).unwrap();
    assert!(matches!(
        db.search_substring(b"hel", None, None),
        Err(UnitError::Storage(kerno_core::Error::Corruption(_)))
    ));
    // A page that does not decode in a list a search reads.
    let (k, _) = pages(&db).into_iter().next().unwrap();
    db.raw_commit(vec![(k, Some(vec![1, 0, 1]))]).unwrap();
    assert!(matches!(
        db.search_substring(b"hel", None, None),
        Err(UnitError::Storage(kerno_core::Error::Corruption(_)))
    ));
}

#[test]
fn a_batch_of_units_indexes_what_one_commit_each_indexes() {
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
    // The same ids (the id source counts), so the same units by trigram, in one run or in
    // the runs of 300 change sets merged in tiers.
    assert_eq!(index_of(&one, "sym"), index_of(&many, "sym"));
    assert_eq!(manifest(&one).runs.len(), 1);
    assert!(manifest(&many).runs.len() < 3 * runs::MERGE_FANOUT);
    assert_clean(&one);
    assert_clean(&many);
}

/// The same units committed in batches of any size, with an update of an old unit among
/// them, are the same index to a reader, and `compact` leaves it in one run without the
/// postings of dead numbers.
#[test]
fn batches_of_any_size_are_the_same_index_and_compact_merges_them() {
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
    assert_eq!(index_of(&one, "sym"), index_of(&many, "sym"));
    assert_clean(&many);

    let ids = found(&one, b"token_");
    assert_eq!(ids.len(), 700);
    for db in [&one, &many] {
        change(db, |cs| {
            let upd = UnitUpdate { payload: b"token_9_moved zzq".to_vec(), schema_version: None, metadata: None };
            cs.update(ids[3], upd, None).unwrap();
            cs.create(new("token_9_fresh zzq")).unwrap();
        });
    }
    assert_eq!(index_of(&one, "sym"), index_of(&many, "sym"));
    assert_eq!(found(&many, b"zzq").len(), 2);
    assert_clean(&many);

    for db in [&one, &many] {
        let before = index_of(db, "sym");
        assert!(manifest(db).runs.iter().map(|r| r.dead).sum::<u64>() > 0);
        db.merge_posting_runs().unwrap();
        let m = manifest(db);
        assert_eq!((m.runs.len(), m.runs[0].dead), (1, 0), "{m:?}");
        assert_eq!(index_of(db, "sym"), before);
        assert_eq!(found(db, b"zzq").len(), 2);
        assert_clean(db);
        // Nothing left to merge: nothing is committed.
        let seq = db.snapshot().unwrap().seq();
        db.merge_posting_runs().unwrap();
        assert_eq!(db.snapshot().unwrap().seq(), seq);
    }
}
