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
    assert_eq!(
        db.raw_get(&block_key("alpha", None)).unwrap(),
        None,
        "the old posting, and its emptied block, are gone"
    );
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

/// The raw write that gives `unit` the posting `p` in the list of `term` (`None`: takes it out),
/// for a list of one block, the tail: the damage the verifier must see.
fn posting_patch(db: &UnitDb<E>, term: &str, unit: UnitId, p: Option<Posting>) -> (Vec<u8>, Option<Vec<u8>>) {
    use crate::blocks::Block;
    let key = block_key(term, None);
    let mut entries = db.raw_get(&key).unwrap().map(|v| Postings::decode(&v).unwrap()).unwrap_or_default();
    entries.retain(|e| e.0 != unit.0);
    if let Some(p) = p {
        entries.push((unit.0, p));
        entries.sort_by_key(|e| e.0);
    }
    (key, (!entries.is_empty()).then(|| Postings::encode(&entries)))
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
    db.raw_commit(vec![posting_patch(&db, "gamma", ids[1], None)]).unwrap();
    assert!(rules(&db).contains(&"UNIT-49".to_string()), "{:?}", db.verify_units().unwrap().findings);
}

#[test]
fn verify_sees_a_changed_term_frequency() {
    let (db, ids) = corpus_db(21);
    db.raw_commit(vec![posting_patch(&db, "gamma", ids[1], Some(Posting { tf: 1, dl: 3 }))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-49"]);
}

#[test]
fn verify_sees_a_lost_doc_and_a_foreign_doc() {
    let (db, ids) = corpus_db(22);
    db.raw_commit(vec![(doc_key(ids[0]), None)]).unwrap();
    assert!(rules(&db).contains(&"UNIT-49".to_string()));
    let (db, _) = corpus_db(23);
    let ghost = UnitId([9; 16]);
    let doc = analyze(b"ghost").unwrap();
    db.raw_commit(vec![(doc_key(ghost), Some(encode_doc(&doc)))]).unwrap();
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
    db.raw_commit(vec![(block_key("delta", None), Some(vec![1, 0, 0]))]).unwrap();
    let r = rules(&db);
    assert!(!r.is_empty() && !r.contains(&"UNIT-49".to_string()), "{r:?}");
}

#[test]
fn verify_names_no_term() {
    let (db, ids) = corpus_db(26);
    db.raw_commit(vec![
        posting_patch(&db, "gamma", ids[1], None),
        (doc_key(UnitId([7; 16])), Some(encode_doc(&analyze(b"zeta").unwrap()))),
    ])
    .unwrap();
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
    // Written before the first statistic: the writer of an older build left
    // no entries. Simulated by removing every full-text record of the first
    // unit and rewriting the statistics to start after it.
    let (db, ids) = corpus_db(27);
    let (first_doc, _) = decode_doc(&db.raw_get(&doc_key(ids[0])).unwrap().unwrap()).unwrap();
    let mut s = decode_stats(&db.raw_get(&STATS_KEY).unwrap().unwrap()).unwrap();
    s.since = db.snapshot().unwrap().get_unit(ids[0]).unwrap().unwrap().updated_seq.0 + 1;
    s.docs -= 1;
    s.total_len -= u64::from(first_doc);
    db.raw_commit(vec![
        (doc_key(ids[0]), None),
        posting_patch(&db, "alpha", ids[0], None),
        posting_patch(&db, "beta", ids[0], None),
        (STATS_KEY.to_vec(), Some(encode_stats(s))),
    ])
    .unwrap();
    assert_eq!(rules(&db), Vec::<String>::new());
    assert!(!found(&db, "beta").contains(&ids[0]), "a legacy unit is not found until rewritten");
    update(&db, ids[0], "alpha beta");
    assert!(found(&db, "beta").contains(&ids[0]));
    assert_clean(&db);
}

// ---------------------------------------------------------- posting blocks

/// A raw `POSTING` record: its key and its decoded entries.
type PostingBlock = (Vec<u8>, Vec<(UnitId, Posting)>);

/// Every raw `POSTING` record of the store.
fn posting_blocks(db: &UnitDb<E>) -> Vec<PostingBlock> {
    use kerno_core::contract::Snapshot;
    let snap = db.raw_snapshot().unwrap();
    let all = snap.scan(Bound::Included(&[TAG_POSTING][..]), Bound::Excluded(&[TAG_DOC][..]), 100_000).unwrap();
    all.into_iter().map(|(k, v)| (k, decode_block(&v).unwrap())).collect()
}

/// The blocks of one term, in key order.
fn blocks_of(db: &UnitDb<E>, term: &str) -> Vec<PostingBlock> {
    let list = list_key(term);
    posting_blocks(db).into_iter().filter(|(k, _)| k.starts_with(&list)).collect()
}

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
fn a_batch_writes_one_record_per_block_not_one_per_posting() {
    let (db, ids) = long_list(40);
    let blocks = blocks_of(&db, "common");
    // 300 postings in blocks of at most 128, cut from the low end: 128 + 128 + 44.
    assert_eq!(blocks.iter().map(|(_, e)| e.len()).collect::<Vec<_>>(), [128, 128, 44]);
    let units: Vec<UnitId> = blocks.iter().flat_map(|(_, e)| e.iter().map(|(u, _)| *u)).collect();
    assert_eq!(units, ids, "ascending across the blocks");
    // The first two blocks are bounded by their last unit, the third is the tail.
    assert_eq!(blocks[0].0, block_key("common", Some(ids[127])));
    assert_eq!(blocks[1].0, block_key("common", Some(ids[255])));
    assert_eq!(blocks[2].0, block_key("common", None));
    // Each `unitN` term is its own list of one posting: 300 + 1 + 2 lists in all.
    assert_eq!(posting_blocks(&db).len(), 3 + 300 + 1);
    assert_eq!(found(&db, "common").len(), 100, "a search is limited, the list is read whole");
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 300);
    assert_clean(&db);
}

#[test]
fn a_posting_changes_and_leaves_a_middle_block_of_a_long_list() {
    let (db, ids) = long_list(41);
    // A unit in the middle block loses the term and another gains a repeat of it.
    update(&db, ids[130], "other");
    update(&db, ids[200], "common common");
    let blocks = blocks_of(&db, "common");
    assert_eq!(blocks.iter().map(|(_, e)| e.len()).collect::<Vec<_>>(), [128, 127, 44]);
    let tf = blocks[1].1.iter().find(|(u, _)| *u == ids[200]).map(|(_, p)| p.tf);
    assert_eq!(tf, Some(2), "a posting is replaced in place");
    assert!(blocks.iter().all(|(_, e)| e.iter().all(|(u, _)| *u != ids[130])));
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 299);
    assert_clean(&db);
    // Emptying a block deletes its record: every unit of the first block goes.
    change(&db, |cs| {
        for id in &ids[..128] {
            cs.delete(*id, None).unwrap();
        }
    });
    let blocks = blocks_of(&db, "common");
    assert_eq!(blocks.iter().map(|(_, e)| e.len()).collect::<Vec<_>>(), [127, 44]);
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 171);
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
    let blocks = blocks_of(&db, "alpha");
    assert_eq!(blocks.len(), 1, "one block, written once");
    assert_eq!(blocks[0].1.iter().map(|(_, p)| p.tf).collect::<Vec<_>>(), [2, 1]);
    assert_clean(&db);
}

#[test]
fn turning_the_index_off_removes_every_block_and_on_writes_them_again() {
    let (db, _) = long_list(43);
    db.set_fulltext("note", false, ChangeMeta::default()).unwrap();
    assert!(posting_blocks(&db).is_empty());
    assert_clean(&db);
    db.set_fulltext("note", true, ChangeMeta::default()).unwrap();
    assert_eq!(blocks_of(&db, "common").len(), 3);
    assert_eq!(db.search_text("common", 1000, None).unwrap().len(), 300);
    assert_clean(&db);
}

#[test]
fn verify_sees_a_block_that_holds_a_unit_outside_its_range() {
    let (db, ids) = long_list(44);
    let blocks = blocks_of(&db, "common");
    // The first block, bounded by its 128th unit, swaps that unit for the 129th.
    let (key, mut entries) = blocks[0].clone();
    entries.pop();
    entries.push(blocks[1].1[0]);
    use crate::blocks::Block;
    let raw: Vec<_> = entries.iter().map(|(u, p)| (u.0, *p)).collect();
    db.raw_commit(vec![(key, Some(Postings::encode(&raw)))]).unwrap();
    assert!(rules(&db).contains(&"UNIT-49".to_string()), "{:?} {}", db.verify_units().unwrap().findings, ids.len());
}

#[test]
fn verify_sees_block_values_that_do_not_decode() {
    let (db, _) = long_list(45);
    let key = block_key("rare", None);
    let good = db.raw_get(&key).unwrap().unwrap();
    for (name, bad, rule) in [
        ("no unit", vec![1, 0], "UNIT-9"),
        ("a partial posting", [&good[..], &[7][..]].concat(), "UNIT-9"),
        ("a newer envelope", [&[2u8][..], &good[1..]].concat(), "UNIT-7"),
        ("reserved flags", [&[1u8, 1][..], &good[2..]].concat(), "UNIT-8"),
        ("a zero frequency", [&good[..18], &[0, 0, 0, 0][..], &good[22..]].concat(), "UNIT-12"),
    ] {
        db.raw_commit(vec![(key.clone(), Some(bad))]).unwrap();
        let r = rules(&db);
        assert!(r.contains(&rule.to_string()) && !r.contains(&"UNIT-49".to_string()), "{name}: {r:?}");
        db.raw_commit(vec![(key.clone(), Some(good.clone()))]).unwrap();
    }
    assert_clean(&db);
    // A key that is not tag, term, NUL, bound.
    db.raw_commit(vec![([&[TAG_POSTING][..], b"rare", &[0, 7]].concat(), Some(good))]).unwrap();
    assert!(rules(&db).contains(&"UNIT-3".to_string()));
}
