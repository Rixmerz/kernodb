//! The document store end to end, with close and reopen, and the demo binary.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use document_store::{Doc, DocumentStore, Error};

/// The tests of this file take turns. `the_demo_runs_to_the_end` starts a process, and a child
/// holds a copy of every file its parent has open until it execs: if a database was open in
/// another test at that moment, its `LOCK` stays held by the child after the test closes it, and
/// opening the directory again fails with `Locked`. A test that opens a database holds a
/// `TempDir`, which holds this guard.
static TURNS: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    TURNS.lock().unwrap_or_else(PoisonError::into_inner)
}

struct TempDir {
    dir: PathBuf,
    _turn: MutexGuard<'static, ()>,
}

impl TempDir {
    fn new() -> Self {
        let turn = take_turn();
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kerno-document-store-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir { dir, _turn: turn }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn doc(slug: &str, author: &str, status: &str, year: i64, body: &str) -> Doc {
    Doc {
        slug: slug.into(),
        title: format!("Title of {slug}"),
        author: author.into(),
        status: status.into(),
        year,
        body: body.into(),
    }
}

fn slugs(docs: &[Doc]) -> Vec<&str> {
    docs.iter().map(|d| d.slug.as_str()).collect()
}

fn open(dir: &TempDir) -> DocumentStore {
    DocumentStore::open(dir.path()).expect("open")
}

#[test]
fn a_document_reads_back_by_slug_after_a_reopen() {
    let dir = TempDir::new();
    let store = open(&dir);
    let d = doc("a", "ann", "draft", 2024, "alpha body");
    store.insert(&d).expect("insert");
    store.close().expect("close");
    let store = open(&dir);
    assert_eq!(store.get("a").expect("get"), Some(d));
    assert_eq!(store.get("missing").expect("get"), None);
    store.close().expect("close");
}

#[test]
fn a_duplicate_slug_is_refused_and_leaves_no_body() {
    let dir = TempDir::new();
    let store = open(&dir);
    store.insert(&doc("a", "ann", "draft", 2024, "alpha")).expect("insert");
    let err = store.insert(&doc("a", "bob", "draft", 2024, "wombat")).expect_err("duplicate");
    assert!(matches!(err, Error::SchemaViolation { .. }), "{err:?}");
    assert!(store.search("wombat", 10).expect("search").is_empty());
    assert!(store.by_author("bob").expect("by author").is_empty());
    store.close().expect("close");
}

#[test]
fn indexed_fields_answer_lookups() {
    let dir = TempDir::new();
    let store = open(&dir);
    store.insert(&doc("b", "ann", "published", 2023, "x")).expect("insert");
    store.insert(&doc("a", "ann", "published", 2023, "x")).expect("insert");
    store.insert(&doc("c", "bob", "published", 2025, "x")).expect("insert");
    store.insert(&doc("d", "bob", "draft", 2024, "x")).expect("insert");

    assert_eq!(slugs(&store.by_author("ann").expect("author")), ["b", "a"]);
    assert_eq!(slugs(&store.by_year(2024, 2025, 10).expect("years")), ["d", "c"]);
    assert_eq!(store.by_year(2023, 2025, 2).expect("limit").len(), 2);
    // Newest year first, then slug.
    assert_eq!(slugs(&store.published().expect("published")), ["c", "a", "b"]);
    store.close().expect("close");
}

#[test]
fn search_returns_the_documents_whose_bodies_match() {
    let dir = TempDir::new();
    let store = open(&dir);
    store.insert(&doc("a", "ann", "draft", 2024, "backup of the data")).expect("insert");
    store.insert(&doc("b", "ann", "draft", 2024, "backup backup backup")).expect("insert");
    store.insert(&doc("c", "ann", "draft", 2024, "nothing relevant")).expect("insert");

    assert_eq!(slugs(&store.search("backup", 10).expect("search")), ["b", "a"]);
    assert_eq!(slugs(&store.search("backup", 1).expect("limit")), ["b"]);
    store.close().expect("close");
}

#[test]
fn a_metadata_match_is_not_a_search_hit() {
    let dir = TempDir::new();
    let store = open(&dir);
    // Year 2048 is 0x0800: every byte of the record is below 0x80, so the typed record is valid
    // UTF-8 and the full-text index reads it. Its words must still not count as hits.
    store.insert(&doc("z", "zebra", "d", 2048, "nothing to see")).expect("insert");
    assert!(store.search("zebra", 10).expect("search").is_empty());
    assert_eq!(slugs(&store.search("nothing", 10).expect("search")), ["z"]);
    store.close().expect("close");
}

#[test]
fn an_update_moves_the_indexes_and_the_text() {
    let dir = TempDir::new();
    let store = open(&dir);
    store.insert(&doc("a", "ann", "draft", 2024, "old zebra text")).expect("insert");
    let new = doc("a", "bob", "published", 2025, "fresh wombat text");
    assert!(store.update(&new).expect("update"));

    assert!(store.by_author("ann").expect("old author").is_empty());
    assert_eq!(slugs(&store.by_author("bob").expect("new author")), ["a"]);
    assert!(store.search("zebra", 10).expect("old word").is_empty());
    assert_eq!(slugs(&store.search("wombat", 10).expect("new word")), ["a"]);
    assert_eq!(store.get("a").expect("get"), Some(new));
    assert!(!store.update(&doc("missing", "x", "d", 1, "y")).expect("missing"));
    store.close().expect("close");
}

#[test]
fn a_deleted_document_is_gone_from_every_lookup() {
    let dir = TempDir::new();
    let store = open(&dir);
    store.insert(&doc("a", "ann", "published", 2024, "wombat text")).expect("insert");
    assert!(store.delete("a").expect("delete"));

    assert_eq!(store.get("a").expect("get"), None);
    assert!(store.by_author("ann").expect("author").is_empty());
    assert!(store.published().expect("published").is_empty());
    assert!(store.search("wombat", 10).expect("search").is_empty());
    assert!(!store.delete("a").expect("delete again"));
    store.insert(&doc("a", "ann", "draft", 2024, "again")).expect("the slug is free again");
    store.close().expect("close");
}

#[test]
fn the_demo_runs_to_the_end() {
    let _turn = take_turn();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_document-store")).output().expect("run the demo");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "demo failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("documents left: 2"), "{stdout}");
}
