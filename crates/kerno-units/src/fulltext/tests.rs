//! Tests of the full-text index on the emulated engine and a simulated disk,
//! so they run wherever the suite does (the `kerno_db` engine refuses
//! `Durability::Sync` on platforms without a verified fsync).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::{CrashKind, SimVfs};

use super::tokenize::analyze;
use super::*;
use crate::{ChangeMeta, CounterIds, FixedClock, NewUnit, RevNo, Timestamp, UnitConfig, UnitUpdate};

type E = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg(first_id: u64) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(first_id)));
    c.engine = Options::small_for_tests();
    c
}

/// The layer, with the keyword index of no type turned on (the default).
fn open_plain(disk: &SimVfs, first_id: u64) -> UnitDb<E> {
    let c = cfg(first_id);
    let engine = E::open(disk.as_vfs(), Path::new("db"), &c.engine).expect("open engine");
    UnitDb::from_engine(engine, &c).expect("open layer")
}

fn open_at(disk: &SimVfs, first_id: u64) -> UnitDb<E> {
    let db = open_plain(disk, first_id);
    // The keyword index is opt-in per type (ADR-0023).
    db.set_fulltext("note", true, crate::ChangeMeta::default()).expect("index notes");
    db
}

fn open(disk: &SimVfs) -> UnitDb<E> {
    open_at(disk, 1)
}

fn new_unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "note".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() }
}

fn store(db: &UnitDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.create(new_unit(payload)).unwrap();
    cs.commit().unwrap();
    id
}

fn update(db: &UnitDb<E>, id: UnitId, text: &str) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload: text.as_bytes().to_vec(), schema_version: None, metadata: None }, None)
        .unwrap();
    cs.commit().unwrap();
}

fn change(db: &UnitDb<E>, f: impl FnOnce(&mut ChangeSet<'_, E>)) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    f(&mut cs);
    cs.commit().unwrap();
}

fn change_with<T>(db: &UnitDb<E>, f: impl FnOnce(&mut ChangeSet<'_, E>) -> T) -> T {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let out = f(&mut cs);
    cs.commit().unwrap();
    out
}

fn found(db: &UnitDb<E>, query: &str) -> Vec<UnitId> {
    db.search_text(query, 100, None).unwrap().into_iter().map(|h| h.unit).collect()
}

fn assert_clean(db: &UnitDb<E>) {
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "verify_units found {:?}", report.findings);
}

fn rules(db: &UnitDb<E>) -> Vec<String> {
    db.verify_units().unwrap().rules().into_iter().collect()
}

// ------------------------------------------------------------ tokenizer

#[test]
fn tokens_are_lowercase_unicode_words() {
    let got: Vec<String> = tokens("Hello, WORLD! ÀÉÎ-ß 42_x\tÜber\nnaïve").collect();
    assert_eq!(got, ["hello", "world", "àéî", "ß", "42", "x", "über", "naïve"]);
    assert_eq!(tokens("  ,;-- ").count(), 0);
    assert_eq!(tokens("").count(), 0);
}

#[test]
fn a_long_term_is_cut_at_a_character_boundary() {
    let ascii = "a".repeat(100);
    assert_eq!(tokens(&ascii).next().unwrap().len(), MAX_TERM_LEN);
    let accents = "é".repeat(40); // 80 bytes
    let t = tokens(&accents).next().unwrap();
    assert_eq!(t, "é".repeat(32));
}

#[test]
fn only_text_with_a_term_is_a_document() {
    assert!(analyze(&[0xff, 0xfe, 0x00]).is_none(), "not UTF-8");
    assert!(analyze(b"... --- !!!").is_none(), "no term");
    assert!(analyze(b"").is_none());
    let doc = analyze(b"a A b").unwrap();
    assert_eq!((doc.len, doc.terms.get("a"), doc.terms.get("b")), (3, Some(&2), Some(&1)));
}

fn record(n: i64, text_len: usize) -> Vec<u8> {
    use crate::schema::{Document, FieldValue};
    let body = format!("alpha {}", "b".repeat(text_len - 6));
    Document::new().with("count", FieldValue::Int(n)).with("body", FieldValue::Text(body)).encode().unwrap()
}

#[test]
fn a_record_that_is_not_utf8_is_indexed_from_its_text_fields() {
    for (n, len) in [(127, 127), (128, 127), (127, 128), (128, 128)] {
        let doc = analyze(&record(n, len)).unwrap_or_else(|| panic!("int {n}, text {len}: not indexed"));
        assert_eq!(doc.terms.get("alpha"), Some(&1), "int {n}, text {len}");
    }
    // A record whose bytes happen to be UTF-8 keeps its raw-text indexing.
    let raw = record(127, 127);
    assert!(std::str::from_utf8(&raw).is_ok());
    assert!(analyze(&raw).unwrap().terms.contains_key("count"));
}

#[test]
fn records_of_any_encoding_are_found_by_search() {
    let disk = SimVfs::new(3);
    let db = open(&disk);
    let ids: Vec<UnitId> =
        [(127, 127), (128, 127), (127, 128), (128, 128)].iter().map(|&(n, l)| store(&db, &record(n, l))).collect();
    let mut got = found(&db, "alpha");
    got.sort();
    let mut want = ids.clone();
    want.sort();
    assert_eq!(got, want);
    assert_clean(&db);
}

// -------------------------------------------------------------- ranking

#[test]
fn one_document_scores_the_closed_form() {
    let disk = SimVfs::new(1);
    let db = open(&disk);
    let a = store(&db, b"solitary");
    // N = 1, n = 1, tf = 1, dl = avgdl: idf = ln(4/3), the tf factor is 1.
    let hits = db.search_text("solitary", 5, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].unit, a);
    assert!((hits[0].score - (4.0f64 / 3.0).ln()).abs() < 1e-12, "score {}", hits[0].score);
}

/// BM25 from the definition, over a corpus read word by word, with none of
/// the module's code.
fn reference(corpus: &[(UnitId, &str)], query: &[&str]) -> Vec<(UnitId, f64)> {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
    };
    let docs: Vec<(UnitId, Vec<String>)> = corpus.iter().map(|(u, t)| (*u, words(t))).collect();
    let n = docs.len() as f64;
    let avgdl = docs.iter().map(|(_, w)| w.len() as f64).sum::<f64>() / n;
    let mut out = Vec::new();
    for (u, w) in &docs {
        let mut score = 0.0;
        for q in query {
            let df = docs.iter().filter(|(_, d)| d.iter().any(|x| x == q)).count() as f64;
            let tf = w.iter().filter(|x| *x == q).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            score += idf * tf * 2.2 / (tf + 1.2 * (0.25 + 0.75 * w.len() as f64 / avgdl));
        }
        if score > 0.0 {
            out.push((*u, score));
        }
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

const CORPUS: [&str; 6] = [
    "The quick brown fox jumps over the lazy dog",
    "quick QUICK quick fox",
    "A lazy dog sleeps all day long in the warm sun while the cat watches the birds",
    "fox",
    "nothing relevant here",
    "Der schnelle braune Fuchs springt über den faulen Hund, naïve café",
];

#[test]
fn ranking_follows_bm25_on_a_small_corpus() {
    let disk = SimVfs::new(2);
    let db = open(&disk);
    let corpus: Vec<(UnitId, &str)> = CORPUS.iter().map(|t| (store(&db, t.as_bytes()), *t)).collect();
    assert_clean(&db);
    for query in [&["quick", "fox"][..], &["lazy", "dog"], &["fox"], &["the"], &["über", "café"], &["absent"]] {
        let want = reference(&corpus, query);
        let got = db.search_text(&query.join(" "), 100, None).unwrap();
        assert_eq!(got.len(), want.len(), "query {query:?}: {got:?} against {want:?}");
        for (g, (u, s)) in got.iter().zip(&want) {
            assert_eq!(g.unit, *u, "query {query:?}: order differs");
            assert!((g.score - s).abs() < 1e-9, "query {query:?}: score {} against {s}", g.score);
        }
    }
    // The shape that matters to a reader: three tf, short text first; a unit
    // with none of the terms is no hit.
    let quick_fox = found(&db, "quick fox");
    assert_eq!(quick_fox.first(), Some(&corpus[1].0));
    assert_eq!(quick_fox.len(), 3);
    assert!(!quick_fox.contains(&corpus[4].0));
    // The query is cut like the text: case and punctuation do not matter.
    assert_eq!(found(&db, "QUICK, fox!"), quick_fox);
    assert_eq!(found(&db, "Über"), vec![corpus[5].0]);
}

#[test]
fn limit_ties_and_empty_queries() {
    let disk = SimVfs::new(3);
    let db = open(&disk);
    let a = store(&db, b"same words");
    let b = store(&db, b"same words");
    let c = store(&db, b"same words same");
    store(&db, b"other");
    let all = db.search_text("same", 10, None).unwrap();
    assert_eq!(all.len(), 3);
    // Equal scores come in unit id order.
    let at = |u: UnitId| all.iter().position(|h| h.unit == u).unwrap();
    assert_eq!(all[at(a)].score, all[at(b)].score);
    assert!(at(a) < at(b), "equal scores: lower unit id first");
    assert!(at(c) < at(a), "a higher tf in a text of the same size ranks first");
    assert_eq!(db.search_text("same", 1, None).unwrap().len(), 1);
    assert_eq!(db.search_text("same", 2, None).unwrap(), all[..2]);
    assert!(db.search_text("missing", 10, None).unwrap().is_empty());
    // A search that could never match is refused, not answered with nothing.
    for (query, limit) in [("same", 0), ("", 10), ("  ,, ", 10)] {
        assert!(matches!(db.search_text(query, limit, None), Err(UnitError::InvalidArgument(_))), "{query:?} {limit}");
    }
}

fn store_as(db: &UnitDb<E>, unit_type: &str, payload: &[u8]) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.create(NewUnit { unit_type: unit_type.into(), ..new_unit(payload) }).unwrap();
    cs.commit().unwrap();
    id
}

#[test]
fn a_search_with_no_keyword_index_is_not_indexed() {
    let disk = SimVfs::new(40);
    let db = open_plain(&disk, 1);
    store_as(&db, "note", b"same words");
    assert!(matches!(db.search_text("same", 10, None), Err(UnitError::NotIndexed { .. })));
    // Restricting to a type does not hide it: the check on the database comes first.
    assert!(matches!(db.search_text("same", 10, Some(&["note"])), Err(UnitError::NotIndexed { .. })));
    db.set_fulltext("note", true, ChangeMeta::default()).unwrap();
    assert_eq!(db.search_text("same", 10, None).unwrap().len(), 1);
    // Off again: no type is indexed once more.
    db.set_fulltext("note", false, ChangeMeta::default()).unwrap();
    assert!(matches!(db.search_text("same", 10, None), Err(UnitError::NotIndexed { .. })));
}

#[test]
fn a_search_restricted_to_types_finds_only_those_types() {
    let disk = SimVfs::new(41);
    let db = open(&disk);
    db.set_fulltext("memo", true, ChangeMeta::default()).unwrap();
    db.set_fulltext("empty_on", true, ChangeMeta::default()).unwrap();
    let note = store_as(&db, "note", b"shared words here");
    let memo = store_as(&db, "memo", b"shared words there");
    let plain = store_as(&db, "plain", b"shared words nowhere"); // never indexed
    let _ = plain;
    let all: Vec<UnitId> = db.search_text("shared", 10, None).unwrap().iter().map(|h| h.unit).collect();
    assert_eq!(all.len(), 2);
    let only = |types: &[&str]| -> Vec<TextHit> { db.search_text("shared", 10, Some(types)).unwrap() };
    assert_eq!(only(&["note"]).iter().map(|h| h.unit).collect::<Vec<_>>(), vec![note]);
    assert_eq!(only(&["memo"]).iter().map(|h| h.unit).collect::<Vec<_>>(), vec![memo]);
    // Several types: the same hits and scores as no restriction.
    let both = only(&["note", "memo"]);
    assert_eq!(both, db.search_text("shared", 10, None).unwrap());
    // A unit scores the same with or without the restriction (whole-index statistics).
    let unrestricted = db.search_text("shared", 10, None).unwrap();
    assert_eq!(only(&["note"])[0].score, unrestricted.iter().find(|h| h.unit == note).unwrap().score);
    // The limit counts after the restriction.
    assert_eq!(db.search_text("shared", 1, Some(&["memo"])).unwrap().len(), 1);
    // A type that is on but holds no match is an empty answer, a real one.
    assert!(only(&["empty_on"]).is_empty());
}

#[test]
fn a_restriction_that_could_never_match_is_refused() {
    let disk = SimVfs::new(42);
    let db = open(&disk);
    store_as(&db, "plain", b"shared words"); // exists, but not keyword-indexed
    store(&db, b"shared words");
    let err = |types: &[&str]| db.search_text("shared", 10, Some(types)).unwrap_err();
    assert!(matches!(err(&["ghost"]), UnitError::UnknownType { unit_type } if unit_type == "ghost"));
    assert!(matches!(err(&["note", "ghost"]), UnitError::UnknownType { .. }), "one unknown type refuses all");
    assert!(matches!(err(&["plain"]), UnitError::NotIndexed { .. }), "a known type that is not indexed");
    assert!(matches!(err(&[]), UnitError::InvalidArgument(_)), "an empty list names no type");
    assert!(matches!(err(&[""]), UnitError::InvalidArgument(_)));
    assert!(matches!(err(&[&"x".repeat(65)]), UnitError::InvalidArgument(_)));
    // A type that was only ever named by its option is known.
    db.set_fulltext("fresh", true, ChangeMeta::default()).unwrap();
    assert!(db.search_text("shared", 10, Some(&["fresh"])).unwrap().is_empty());
}

#[test]
fn a_database_without_text_answers_nothing_and_writes_nothing() {
    let disk = SimVfs::new(4);
    let db = open(&disk);
    assert!(db.search_text("anything", 5, None).unwrap().is_empty());
    let binary = store(&db, &[0xff, 0xfe, 0xfd]);
    let blank = store(&db, b" ... ");
    for id in [binary, blank] {
        assert_eq!(db.raw_get(&doc_key(id)).unwrap(), None);
    }
    assert_eq!(db.raw_get(&STATS_KEY).unwrap(), None, "no text, no statistics record");
    assert!(db.search_text("anything", 5, None).unwrap().is_empty());
    assert_clean(&db);
}

// ----------------------------------------------------------------- sync

#[test]
fn the_index_follows_update_archive_restore_and_delete() {
    let disk = SimVfs::new(5);
    let db = open(&disk);
    let a = store(&db, b"alpha beta");
    let b = store(&db, b"beta gamma");
    assert_eq!(found(&db, "beta").len(), 2);
    let before = db.snapshot().unwrap();

    update(&db, a, "delta");
    assert_eq!(found(&db, "beta"), vec![b]);
    assert!(found(&db, "alpha").is_empty());
    assert_eq!(found(&db, "delta"), vec![a]);
    assert!(!index(&db).contains_key("alpha"), "the old posting is not read any more");
    assert_clean(&db);
    // A snapshot keeps the index it was taken at.
    assert_eq!(before.search_text("alpha", 5, None).unwrap().len(), 1);

    change(&db, |cs| {
        cs.archive(b, None).unwrap();
    });
    assert!(found(&db, "beta").is_empty(), "an archived unit is not found");
    assert!(found(&db, "gamma").is_empty());
    assert_clean(&db);
    change(&db, |cs| {
        cs.unarchive(b, None).unwrap();
    });
    assert_eq!(found(&db, "gamma"), vec![b]);
    assert_clean(&db);

    // Restoring revision 1 brings back the text of revision 1.
    change(&db, |cs| {
        cs.restore(a, RevNo(1), None).unwrap();
    });
    assert_eq!(found(&db, "alpha"), vec![a]);
    assert!(found(&db, "delta").is_empty());
    assert_clean(&db);

    change(&db, |cs| {
        cs.delete(a, None).unwrap();
    });
    assert!(found(&db, "alpha").is_empty());
    assert_eq!(found(&db, "beta"), vec![b]);
    change(&db, |cs| {
        cs.delete(b, None).unwrap();
    });
    assert!(found(&db, "beta").is_empty());
    assert_clean(&db);
    // Nothing of any unit is left but the statistics of an empty index.
    let stats = decode_stats(&db.raw_get(&STATS_KEY).unwrap().unwrap()).unwrap();
    assert_eq!((stats.docs, stats.total_len), (0, 0));
}

#[test]
fn text_turning_into_bytes_leaves_the_index() {
    let disk = SimVfs::new(6);
    let db = open(&disk);
    let a = store(&db, b"word");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(a, UnitUpdate { payload: vec![0xff, 0xfe], schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    assert!(found(&db, "word").is_empty());
    assert_eq!(db.raw_get(&doc_key(a)).unwrap(), None);
    assert_clean(&db);
}

#[test]
fn several_units_in_one_change_set_add_up() {
    let disk = SimVfs::new(7);
    let db = open(&disk);
    let mut ids = Vec::new();
    change(&db, |cs| {
        for t in ["one two", "two three three", "four"] {
            ids.push(cs.create(new_unit(t.as_bytes())).unwrap());
        }
    });
    // Each create read the statistics the one before it wrote.
    assert_clean(&db);
    let stats = decode_stats(&db.raw_get(&STATS_KEY).unwrap().unwrap()).unwrap();
    assert_eq!((stats.docs, stats.total_len), (3, 6));
    assert_eq!(found(&db, "three"), vec![ids[1]]);
}

#[test]
fn an_aborted_change_set_leaves_no_entry() {
    let disk = SimVfs::new(8);
    let db = open(&disk);
    let a = store(&db, b"kept");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(a, UnitUpdate { payload: b"lost".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.create(new_unit(b"lost too")).unwrap();
    cs.abort();
    assert_eq!(found(&db, "kept"), vec![a]);
    assert!(found(&db, "lost").is_empty());
    assert_clean(&db);
}

// ------------------------------------------------------------- recovery

fn snapshot_of(db: &UnitDb<E>) -> BTreeMap<&'static str, Vec<TextHit>> {
    ["quick fox", "lazy", "the", "über"].into_iter().map(|q| (q, db.search_text(q, 100, None).unwrap())).collect()
}

#[test]
fn the_index_survives_a_clean_reopen_and_a_crash() {
    let disk = SimVfs::new(9);
    let db = open(&disk);
    let ids: Vec<UnitId> = CORPUS.iter().map(|t| store(&db, t.as_bytes())).collect();
    update(&db, ids[0], "quick quick lazy");
    change(&db, |cs| {
        cs.delete(ids[3], None).unwrap();
    });
    let want = snapshot_of(&db);
    assert!(want.values().all(|h| !h.is_empty()));
    db.close().unwrap();

    let db = open_at(&disk, 100);
    assert_eq!(snapshot_of(&db), want, "same hits and the same scores after a reopen");
    assert_clean(&db);
    // It keeps being maintained after the reopen.
    let fresh = store(&db, b"quick lazy newcomer");
    assert!(found(&db, "newcomer").contains(&fresh));
    let want = snapshot_of(&db);
    // An unclean end: the handle is dropped without closing, then power is lost.
    let after = disk.crash_now(CrashKind::PowerLoss);
    drop(db);
    let db = open_at(&after, 200);
    assert_eq!(snapshot_of(&db), want, "acknowledged commits are indexed after a crash");
    assert_clean(&db);
    db.close().unwrap();
}

#[test]
fn export_and_import_rebuild_the_index() {
    let disk = SimVfs::new(10);
    let db = open(&disk);
    let ids: Vec<UnitId> = CORPUS.iter().map(|t| store(&db, t.as_bytes())).collect();
    update(&db, ids[1], "quick quick");
    change(&db, |cs| {
        cs.archive(ids[4], None).unwrap();
    });
    let want = snapshot_of(&db);
    let bytes = crate::export::export(&db).unwrap();

    let other = SimVfs::new(11);
    let copy = open_plain(&other, 1);
    // The import ends with `verify_snapshot`, which includes UNIT-49. The
    // export carries no type option (export-v1 §1, like schemas), so the
    // index is turned on after the import and indexes the imported units.
    crate::import::import(&copy, &bytes).unwrap();
    copy.set_fulltext("note", true, crate::ChangeMeta::default()).unwrap();
    assert_eq!(snapshot_of(&copy), want);
    assert_clean(&copy);
}

// --------------------------------------------------------------- verify

/// The index as a search reads it: the units of each term with their postings, in id order.
fn index(db: &UnitDb<E>) -> BTreeMap<String, Vec<(UnitId, Posting)>> {
    let snap = db.raw_snapshot().unwrap();
    runs::dump_units::<_, Postings>(&snap, &PREFIX)
        .into_iter()
        .map(|(list, units)| {
            let term = String::from_utf8(list[..list.len() - 1].to_vec()).unwrap();
            (term, units.into_iter().map(|(u, tf, dl)| (u, Posting { tf, dl })).collect())
        })
        .collect()
}

fn manifest(db: &UnitDb<E>) -> runs::Manifest {
    use kerno_core::contract::Snapshot;
    let snap = db.raw_snapshot().unwrap();
    runs::read_manifest::<Postings>(&|k| snap.get(k), &PREFIX).unwrap()
}

/// The number and the document length of `unit` in the index.
fn number_of(db: &UnitDb<E>, unit: UnitId) -> (u64, u32) {
    runs::decode_doc(&db.raw_get(&doc_key(unit)).unwrap().expect("a document record"), "DOC").unwrap()
}

/// Every run page of the index, as `(key, value)`.
fn pages(db: &UnitDb<E>) -> Vec<(Vec<u8>, Vec<u8>)> {
    use kerno_core::contract::Snapshot;
    let snap = db.raw_snapshot().unwrap();
    let (from, to) = (runs::kind_key(&PREFIX, runs::KIND_RUN), runs::kind_key(&PREFIX, runs::KIND_RUN + 1));
    snap.scan(Bound::Included(&from), Bound::Excluded(&to), 100_000).unwrap()
}

/// The raw writes that give `unit` the posting `p` in the list of `term` (`None`: take it out),
/// in pages the manifest agrees with: the damage only a comparison with the text can see.
fn posting_patch(db: &UnitDb<E>, term: &str, unit: UnitId, p: Option<Posting>) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    let (num, _) = number_of(db, unit);
    runs::forge::<_, Postings>(&db.raw_snapshot().unwrap(), &PREFIX, |lists| {
        let list = lists.entry(list_key(term)).or_default();
        list.retain(|e| e.0 != num);
        if let Some(p) = p {
            list.push((num, p.tf, p.dl));
            list.sort_unstable();
        }
        if list.is_empty() {
            lists.remove(&list_key(term));
        }
    })
}

fn corpus_db(seed: u64) -> (UnitDb<E>, Vec<UnitId>) {
    let disk = SimVfs::new(seed);
    let db = open(&disk);
    let ids = ["alpha beta", "beta gamma gamma", "delta"].iter().map(|t| store(&db, t.as_bytes())).collect();
    assert_clean(&db);
    (db, ids)
}

#[test]
fn verify_sees_a_lost_posting() {
    let (db, ids) = corpus_db(20);
    db.raw_commit(posting_patch(&db, "gamma", ids[1], None)).unwrap();
    assert_eq!(rules(&db), ["UNIT-49"], "{:?}", db.verify_units().unwrap().findings);
}

#[test]
fn verify_sees_a_changed_term_frequency() {
    let (db, ids) = corpus_db(21);
    db.raw_commit(posting_patch(&db, "gamma", ids[1], Some(Posting { tf: 1, dl: 3 }))).unwrap();
    assert_eq!(rules(&db), ["UNIT-49"]);
}

#[test]
fn verify_sees_a_lost_doc_and_a_foreign_doc() {
    let (db, ids) = corpus_db(22);
    db.raw_commit(vec![(doc_key(ids[0]), None)]).unwrap();
    assert!(rules(&db).contains(&"UNIT-49".to_string()));
    let (db, _) = corpus_db(23);
    let ghost = UnitId([9; 16]);
    db.raw_commit(vec![(doc_key(ghost), Some(runs::encode_doc(0, 1)))]).unwrap();
    assert!(rules(&db).contains(&"UNIT-49".to_string()));
}

#[test]
fn verify_sees_wrong_statistics() {
    let (db, _) = corpus_db(24);
    let mut s = decode_stats(&db.raw_get(&STATS_KEY).unwrap().unwrap()).unwrap();
    s.docs -= 1;
    db.raw_commit(vec![(STATS_KEY.to_vec(), Some(encode_stats(s)))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-49"]);
    // Without a statistics record, entries have no author.
    db.raw_commit(vec![(STATS_KEY.to_vec(), None)]).unwrap();
    assert_eq!(rules(&db), ["UNIT-49"]);
}

#[test]
fn a_record_that_does_not_decode_is_a_structure_finding_alone() {
    let (db, _) = corpus_db(25);
    let (key, _) = pages(&db).pop().unwrap();
    db.raw_commit(vec![(key, Some(vec![1, 0, 0]))]).unwrap();
    let r = rules(&db);
    assert!(!r.is_empty() && !r.contains(&"UNIT-49".to_string()), "{r:?}");
}

#[test]
fn verify_names_no_term() {
    let (db, ids) = corpus_db(26);
    db.raw_commit(posting_patch(&db, "gamma", ids[1], None)).unwrap();
    db.raw_commit(vec![(doc_key(UnitId([7; 16])), Some(runs::encode_doc(9, 1)))]).unwrap();
    let report = db.verify_units().unwrap();
    assert!(!report.findings.is_empty());
    for f in &report.findings {
        let text = format!("{} {:?}", f.detail, f.key);
        for secret in ["alpha", "beta", "gamma", "delta", "zeta"] {
            assert!(!text.contains(secret), "a finding carries the term {secret:?}: {text}");
        }
    }
}

#[test]
fn a_unit_older_than_the_index_is_not_a_finding() {
    // Written before the first statistic: nothing indexed it. Simulated by taking the first
    // unit out of the index as the writer would (its number dead, no document record, no
    // posting) and rewriting the statistics to start after it.
    let (db, ids) = corpus_db(27);
    let (num, first_doc) = number_of(&db, ids[0]);
    let mut s = decode_stats(&db.raw_get(&STATS_KEY).unwrap().unwrap()).unwrap();
    s.since = db.snapshot().unwrap().get_unit(ids[0]).unwrap().unwrap().updated_seq.0 + 1;
    s.docs -= 1;
    s.total_len -= u64::from(first_doc);
    let page = runs::num_key(&PREFIX, 0);
    let mut slots = runs::decode_num(&db.raw_get(&page).unwrap().unwrap(), "POSTING").unwrap();
    slots[num as usize].0 = false;
    db.raw_commit(vec![
        (doc_key(ids[0]), None),
        (page, Some(runs::encode_num(&slots))),
        (STATS_KEY.to_vec(), Some(encode_stats(s))),
    ])
    .unwrap();
    let forged = runs::forge::<_, Postings>(&db.raw_snapshot().unwrap(), &PREFIX, |lists| {
        lists.values_mut().for_each(|l| l.retain(|p| p.0 != num));
        lists.retain(|_, l| !l.is_empty());
    });
    db.raw_commit(forged).unwrap();
    assert_eq!(rules(&db), Vec::<String>::new());
    assert!(!found(&db, "beta").contains(&ids[0]), "a legacy unit is not found until rewritten");
    update(&db, ids[0], "alpha beta");
    assert!(found(&db, "beta").contains(&ids[0]));
    assert_clean(&db);
}

// ------------------------------------------------------------ posting runs

/// 300 units that hold "common", written in one change set, and one more that holds "rare".
fn long_list(seed: u64) -> (UnitDb<E>, Vec<UnitId>) {
    let disk = SimVfs::new(seed);
    let db = open(&disk);
    let mut ids = Vec::new();
    change(&db, |cs| {
        for i in 0..300 {
            ids.push(cs.create(new_unit(format!("common unit{i}").as_bytes())).unwrap());
        }
    });
    store(&db, b"rare");
    (db, ids)
}

#[test]
fn a_batch_writes_one_run_of_pages_not_one_record_per_posting() {
    let (db, ids) = long_list(40);
    let m = manifest(&db);
    // Two change sets, two runs: 300 numbers and then one.
    assert_eq!(m.next, 301);
    assert_eq!(m.runs.iter().map(|r| (r.min, r.max, r.level, r.dead)).collect::<Vec<_>>(), [(0, 299, 0, 0), (300, 300, 0, 0)]);
    assert_eq!(m.runs.iter().map(|r| r.postings).collect::<Vec<_>>(), [600, 1]);
    let all = pages(&db);
    assert_eq!(all.len() as u64, m.runs.iter().map(|r| r.pages).sum::<u64>());
    assert!(all.len() < 150, "{} pages for 601 postings", all.len());
    // The list of "common" is cut across several pages of the first run, each keyed by the
    // list and the number of its last posting.
    let common: Vec<&(Vec<u8>, Vec<u8>)> = all.iter().filter(|(k, _)| k[10..].starts_with(b"common\0")).collect();
    assert!(common.len() > 2);
    let run = [&[TAG_POSTING, 3][..], &m.runs[0].id.to_be_bytes()].concat();
    assert!(common.iter().all(|(k, _)| k.len() == 2 + 8 + 7 + 8 && k[..10] == run[..]));
    let units: Vec<UnitId> = index(&db)["common"].iter().map(|(u, _)| *u).collect();
    assert_eq!(units, ids);
    assert_eq!(found(&db, "common").len(), 100, "a search is limited, the list is read whole");
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 300);
    assert_clean(&db);
}

#[test]
fn a_posting_changes_and_leaves_the_middle_of_a_long_list() {
    let (db, ids) = long_list(41);
    // A unit in the middle of the list loses the term: it gets a new number in a new run,
    // and the old one dies where it is.
    update(&db, ids[130], "other");
    let m = manifest(&db);
    assert_eq!((m.next, m.live, m.runs.len(), m.runs[0].dead), (302, 301, 3, 1));
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 299);
    assert_clean(&db);
    // Another gains a repeat of it. That change set is the fourth run: the tier is full, so
    // it writes the four as one run of the next level, without the dead numbers.
    assert_eq!(runs::MERGE_FANOUT, 4);
    update(&db, ids[200], "common common");
    let m = manifest(&db);
    assert_eq!((m.next, m.live, m.runs.len()), (303, 301, 1));
    assert_eq!((m.runs[0].level, m.runs[0].dead, m.runs[0].min, m.runs[0].max), (1, 0, 0, 302));
    let common = &index(&db)["common"];
    assert_eq!(common.len(), 299);
    assert_eq!(common.iter().find(|(u, _)| *u == ids[200]).map(|(_, p)| p.tf), Some(2));
    assert!(common.iter().all(|(u, _)| *u != ids[130]));
    assert_eq!(number_of(&db, ids[200]), (302, 2));
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 299);
    assert_clean(&db);
    // Units that leave only mark their numbers dead: no run is written.
    change(&db, |cs| {
        for id in &ids[..128] {
            cs.delete(*id, None).unwrap();
        }
    });
    let m = manifest(&db);
    assert_eq!((m.live, m.runs.len(), m.runs[0].dead), (173, 1, 128));
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 171);
    assert_clean(&db);
    // A merge drops the postings of the dead numbers; the answer is the same.
    let before = index(&db);
    assert_eq!(db.merge_posting_runs().unwrap(), 1);
    let m = manifest(&db);
    assert_eq!((m.runs.len(), m.runs[0].dead, m.runs[0].min, m.runs[0].max), (1, 0, 0, 302));
    assert_eq!(m.runs[0].postings as usize, before.values().map(Vec::len).sum::<usize>());
    assert_eq!(index(&db), before);
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 171);
    assert_eq!(db.merge_posting_runs().unwrap(), 0, "nothing left to merge");
    assert_clean(&db);
}

#[test]
fn a_create_and_an_update_that_share_a_list_in_one_change_set_both_land() {
    let disk = SimVfs::new(42);
    let db = open(&disk);
    let a = store(&db, b"alpha beta");
    let b = change_with(&db, |cs| {
        let b = cs.create(new_unit(b"alpha gamma")).unwrap();
        cs.update(a, UnitUpdate { payload: b"beta alpha alpha".to_vec(), schema_version: None, metadata: None }, None)
            .unwrap();
        b
    });
    let hits = db.search_text("alpha", 5, None).unwrap();
    assert_eq!(hits.iter().map(|h| h.unit).collect::<std::collections::BTreeSet<_>>(), [a, b].into());
    assert_eq!(index(&db)["alpha"].iter().map(|(u, p)| (*u, p.tf)).collect::<Vec<_>>(), [(a, 2), (b, 1)]);
    assert_clean(&db);
    // A unit indexed twice in one change set (its type turns the index on, then it is
    // rewritten): its first number is dead before it is stored, and its postings are not
    // written.
    let memo = NewUnit { unit_type: "memo".into(), schema_version: 1, metadata: vec![], payload: b"alpha first".to_vec() };
    let c = change_with(&db, |cs| cs.create(memo).unwrap());
    change(&db, |cs| {
        cs.set_fulltext("memo", true).unwrap();
        cs.update(c, UnitUpdate { payload: b"alpha second".to_vec(), schema_version: None, metadata: None }, None)
            .unwrap();
    });
    assert!(!index(&db).contains_key("first"));
    assert_eq!(found(&db, "second"), [c]);
    let m = manifest(&db);
    assert_eq!((m.next, m.live), (5, 3));
    assert!(m.runs.iter().all(|r| r.dead <= 1), "{m:?}");
    assert_clean(&db);
}

#[test]
fn turning_the_index_off_removes_every_record_and_on_writes_them_again() {
    use kerno_core::contract::Snapshot;
    let (db, _) = long_list(43);
    db.set_fulltext("note", false, ChangeMeta::default()).unwrap();
    let snap = db.raw_snapshot().unwrap();
    let left = snap.scan(Bound::Included(&[TAG_POSTING][..]), Bound::Excluded(&[TAG_STATS][..]), 10).unwrap();
    assert!(left.is_empty(), "an index that holds nothing has no record: {left:?}");
    assert_clean(&db);
    db.set_fulltext("note", true, ChangeMeta::default()).unwrap();
    assert_eq!(manifest(&db).runs.len(), 1);
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 300);
    assert_clean(&db);
}

/// Each record of the index lost alone: the verifier reports it, and a search that needs a
/// lost page is refused instead of answering short.
#[test]
fn verify_and_a_search_see_a_lost_record() {
    let (db, ids) = long_list(44);
    update(&db, ids[7], "common again");
    let corrupt = |r: &Result<Vec<TextHit>, UnitError>| matches!(r, Err(UnitError::Storage(kerno_core::Error::Corruption(_))));
    let mut refused = 0;
    for (k, v) in pages(&db) {
        db.raw_commit(vec![(k.clone(), None)]).unwrap();
        assert_eq!(rules(&db), ["UNIT-49"], "page {k:?}");
        for (query, want) in [("common", 300), ("unit150", 1), ("rare", 1), ("again", 1), ("unit0", 1), ("unit99", 1)] {
            let r = db.search_text(query, 1000, None);
            if corrupt(&r) {
                refused += 1;
            } else {
                assert_eq!(r.unwrap().len(), want, "page {k:?}, query {query:?}: a short answer");
            }
        }
        db.raw_commit(vec![(k, Some(v))]).unwrap();
        assert_clean(&db);
    }
    assert!(refused > 0, "a search that needs a lost page is refused");
    for k in [runs::manifest_key(&PREFIX), doc_key(ids[5]), runs::num_key(&PREFIX, 0), runs::num_key(&PREFIX, 1)] {
        let v = db.raw_get(&k).unwrap().expect("the record exists");
        db.raw_commit(vec![(k.clone(), None)]).unwrap();
        assert!(rules(&db).contains(&"UNIT-49".to_string()), "key {k:?}");
        db.raw_commit(vec![(k, Some(v))]).unwrap();
        assert_clean(&db);
    }
    // A page of a run the manifest does not list.
    let (k, v) = pages(&db).remove(0);
    let mut stray = k.clone();
    stray[2..10].copy_from_slice(&9_999u64.to_be_bytes());
    db.raw_commit(vec![(stray, Some(v))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-49"]);
}

#[test]
fn verify_sees_values_that_do_not_decode() {
    let (db, _) = long_list(45);
    // The page of the run of one posting: "rare", number 300, frequency 1, length 1.
    let (key, good) = pages(&db).pop().unwrap();
    assert_eq!(key, [&[TAG_POSTING, 3][..], &manifest(&db).runs[1].id.to_be_bytes(), b"rare\0", &300u64.to_be_bytes()].concat());
    // envelope, base 300 (two bytes), no previous key, the list, one posting of three bytes.
    assert_eq!(good, [&[1u8, 0, 0xAC, 0x02, 0][..], b"rare\0", &[1, 3, 0, 1, 1]].concat());
    let n = good.len();
    for (name, bad, rule) in [
        ("no header", vec![1, 0], "UNIT-9"),
        ("no item", good[..5].to_vec(), "UNIT-9"),
        ("a cut item", good[..n - 1].to_vec(), "UNIT-9"),
        ("a trailing byte in an item", [&good[..n - 4], &[4, 0, 1, 1, 0][..]].concat(), "UNIT-10"),
        ("a newer envelope", [&[2u8][..], &good[1..]].concat(), "UNIT-7"),
        ("reserved flags", [&[1u8, 1][..], &good[2..]].concat(), "UNIT-8"),
        ("a zero frequency", [&good[..n - 2], &[0, 1][..]].concat(), "UNIT-12"),
        ("a frequency above the length", [&good[..n - 2], &[2, 1][..]].concat(), "UNIT-12"),
        ("an integer not in its shortest form", [&good[..n - 3], &[0x80, 0, 1, 1][..]].concat(), "UNIT-9"),
        ("a number that is not the key's", [&good[..n - 3], &[1, 1, 1][..]].concat(), "UNIT-12"),
    ] {
        db.raw_commit(vec![(key.clone(), Some(bad))]).unwrap();
        let r = rules(&db);
        assert!(r.contains(&rule.to_string()) && !r.contains(&"UNIT-49".to_string()), "{name}: {r:?}");
        db.raw_commit(vec![(key.clone(), Some(good.clone()))]).unwrap();
    }
    assert_clean(&db);
    // Keys that are no record of the index: an unknown kind, a run page with no number, a
    // term with no NUL.
    for bad_key in [
        vec![TAG_POSTING, 9],
        [&[TAG_POSTING, 3][..], &[0u8; 8], b"rare\0", &[0u8; 7]].concat(),
        [&[TAG_POSTING, 3][..], &[0u8; 8], b"rare", &[1u8; 8]].concat(),
    ] {
        db.raw_commit(vec![(bad_key.clone(), Some(good.clone()))]).unwrap();
        assert!(rules(&db).contains(&"UNIT-3".to_string()), "key {bad_key:?}");
        db.raw_commit(vec![(bad_key, None)]).unwrap();
    }
    assert_clean(&db);
    // The manifest, a document record and a number page, each damaged alone.
    let unit = found(&db, "rare")[0];
    for key in [runs::manifest_key(&PREFIX), doc_key(unit), runs::num_key(&PREFIX, 1)] {
        let good = db.raw_get(&key).unwrap().unwrap();
        for bad in [vec![1, 0], vec![2, 0], [&good[..], &[0u8]].concat(), good[..good.len() - 1].to_vec()] {
            db.raw_commit(vec![(key.clone(), Some(bad.clone()))]).unwrap();
            let r = rules(&db);
            assert!(!r.is_empty() && !r.contains(&"UNIT-49".to_string()), "key {key:?} value {bad:?}: {r:?}");
        }
        db.raw_commit(vec![(key, Some(good))]).unwrap();
        assert_clean(&db);
    }
}
