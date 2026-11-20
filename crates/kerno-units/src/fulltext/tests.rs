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

fn open_at(disk: &SimVfs, first_id: u64) -> UnitDb<E> {
    let c = cfg(first_id);
    let engine = E::open(disk.as_vfs(), Path::new("db"), &c.engine).expect("open engine");
    UnitDb::from_engine(engine, &c).expect("open layer")
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

fn found(db: &UnitDb<E>, query: &str) -> Vec<UnitId> {
    db.search_text(query, 100).unwrap().into_iter().map(|h| h.unit).collect()
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
    let hits = db.search_text("solitary", 5).unwrap();
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
        let got = db.search_text(&query.join(" "), 100).unwrap();
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
    let all = db.search_text("same", 10).unwrap();
    assert_eq!(all.len(), 3);
    // Equal scores come in unit id order.
    let at = |u: UnitId| all.iter().position(|h| h.unit == u).unwrap();
    assert_eq!(all[at(a)].score, all[at(b)].score);
    assert!(at(a) < at(b), "equal scores: lower unit id first");
    assert!(at(c) < at(a), "a higher tf in a text of the same size ranks first");
    assert_eq!(db.search_text("same", 1).unwrap().len(), 1);
    assert_eq!(db.search_text("same", 2).unwrap(), all[..2]);
    assert!(db.search_text("same", 0).unwrap().is_empty());
    assert!(db.search_text("", 10).unwrap().is_empty());
    assert!(db.search_text("  ,, ", 10).unwrap().is_empty());
    assert!(db.search_text("missing", 10).unwrap().is_empty());
}

#[test]
fn a_database_without_text_answers_nothing_and_writes_nothing() {
    let disk = SimVfs::new(4);
    let db = open(&disk);
    assert!(db.search_text("anything", 5).unwrap().is_empty());
    let binary = store(&db, &[0xff, 0xfe, 0xfd]);
    let blank = store(&db, b" ... ");
    for id in [binary, blank] {
        assert_eq!(db.raw_get(&doc_key(id)).unwrap(), None);
    }
    assert_eq!(db.raw_get(&STATS_KEY).unwrap(), None, "no text, no statistics record");
    assert!(db.search_text("anything", 5).unwrap().is_empty());
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
    assert_eq!(db.raw_get(&posting_key("alpha", a)).unwrap(), None, "the old posting is gone");
    assert_clean(&db);
    // A snapshot keeps the index it was taken at.
    assert_eq!(before.search_text("alpha", 5).unwrap().len(), 1);

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
    ["quick fox", "lazy", "the", "über"].into_iter().map(|q| (q, db.search_text(q, 100).unwrap())).collect()
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
    let copy = open(&other);
    // The import ends with `verify_snapshot`, which includes UNIT-49.
    crate::import::import(&copy, &bytes).unwrap();
    assert_eq!(snapshot_of(&copy), want);
    assert_clean(&copy);
}

// --------------------------------------------------------------- verify

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
    db.raw_commit(vec![(posting_key("gamma", ids[1]), None)]).unwrap();
    assert!(rules(&db).contains(&"UNIT-49".to_string()), "{:?}", db.verify_units().unwrap().findings);
}

#[test]
fn verify_sees_a_changed_term_frequency() {
    let (db, ids) = corpus_db(21);
    db.raw_commit(vec![(posting_key("gamma", ids[1]), Some(encode_posting(Posting { tf: 1, dl: 3 })))]).unwrap();
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
    let (db, ids) = corpus_db(25);
    db.raw_commit(vec![(posting_key("delta", ids[2]), Some(vec![1, 0, 0]))]).unwrap();
    let r = rules(&db);
    assert!(!r.is_empty() && !r.contains(&"UNIT-49".to_string()), "{r:?}");
}

#[test]
fn verify_names_no_term() {
    let (db, ids) = corpus_db(26);
    db.raw_commit(vec![
        (posting_key("gamma", ids[1]), None),
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
        (posting_key("alpha", ids[0]), None),
        (posting_key("beta", ids[0]), None),
        (STATS_KEY.to_vec(), Some(encode_stats(s))),
    ])
    .unwrap();
    assert_eq!(rules(&db), Vec::<String>::new());
    assert!(!found(&db, "beta").contains(&ids[0]), "a legacy unit is not found until rewritten");
    update(&db, ids[0], "alpha beta");
    assert!(found(&db, "beta").contains(&ids[0]));
    assert_clean(&db);
}
