//! The chat history end to end, with close and reopen, and the demo binary.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use chat_history::{ChatHistory, Error, Role, UnitId};

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
            "kerno-chat-history-test-{}-{}",
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

fn texts(turns: &[chat_history::Turn]) -> Vec<&str> {
    turns.iter().map(|t| t.text.as_str()).collect()
}

fn fill(chat: &ChatHistory, session: UnitId, n: usize) -> Vec<UnitId> {
    (1..=n).map(|i| chat.append(session, Role::User, &format!("turn {i}")).expect("append")).collect()
}

#[test]
fn pages_go_back_from_the_newest_turn_after_a_reopen() {
    let dir = TempDir::new();
    let chat = ChatHistory::open(dir.path()).expect("open");
    let s = chat.session("a").expect("session");
    fill(&chat, s, 7);
    chat.close().expect("close");
    let chat = ChatHistory::open(dir.path()).expect("reopen");

    let p1 = chat.recent(s, 3, None).expect("page 1");
    assert_eq!(texts(&p1.turns), ["turn 5", "turn 6", "turn 7"]);
    let p2 = chat.recent(s, 3, p1.older).expect("page 2");
    assert_eq!(texts(&p2.turns), ["turn 2", "turn 3", "turn 4"]);
    assert!(p2.older.is_some());
    let p3 = chat.recent(s, 3, p2.older).expect("page 3");
    assert_eq!(texts(&p3.turns), ["turn 1"]);
    assert_eq!(p3.older, None);
    // A page that ends exactly at the oldest turn has nothing older.
    assert_eq!(chat.recent(s, 7, None).expect("all").older, None);
    assert!(chat.recent(s, 6, None).expect("six").older.is_some());
    chat.close().expect("close");
}

#[test]
fn neither_a_page_nor_a_search_holds_another_sessions_turns() {
    let dir = TempDir::new();
    let chat = ChatHistory::open(dir.path()).expect("open");
    let (a, b) = (chat.session("a").expect("a"), chat.session("b").expect("b"));
    let a1 = chat.append(a, Role::User, "backup in a").expect("append");
    chat.append(b, Role::User, "backup in b").expect("append");
    let a2 = chat.append(a, Role::Assistant, "another backup in a").expect("append");

    let ids: Vec<UnitId> = chat.recent(a, 10, None).expect("recent").turns.iter().map(|t| t.id).collect();
    assert_eq!(ids, [a1, a2]);
    let mut hits: Vec<UnitId> = chat.search(a, "backup", 10).expect("search").iter().map(|t| t.id).collect();
    hits.sort();
    assert_eq!(hits, [a1, a2]);
    assert_eq!(chat.search(b, "backup", 10).expect("search").len(), 1);
    chat.close().expect("close");
}

#[test]
fn a_cursor_from_another_session_is_refused() {
    let dir = TempDir::new();
    let chat = ChatHistory::open(dir.path()).expect("open");
    let (a, b) = (chat.session("a").expect("a"), chat.session("b").expect("b"));
    let b_turn = chat.append(b, Role::User, "hello").expect("append");
    assert!(matches!(chat.recent(a, 3, Some(b_turn)), Err(Error::InvalidArgument(_))));
    chat.close().expect("close");
}

#[test]
fn a_limit_of_zero_is_refused() {
    let dir = TempDir::new();
    let chat = ChatHistory::open(dir.path()).expect("open");
    let a = chat.session("a").expect("a");
    assert!(matches!(chat.recent(a, 0, None), Err(Error::InvalidArgument(_))));
    chat.close().expect("close");
}

#[test]
fn search_ranks_the_turn_with_more_matches_first() {
    let dir = TempDir::new();
    let chat = ChatHistory::open(dir.path()).expect("open");
    let a = chat.session("a").expect("a");
    chat.append(a, Role::User, "the backup failed").expect("append");
    chat.append(a, Role::User, "backup backup again").expect("append");
    chat.append(a, Role::User, "nothing here").expect("append");

    let hits = chat.search(a, "backup", 10).expect("search");
    assert_eq!(texts(&hits), ["backup backup again", "the backup failed"]);
    assert_eq!(chat.search(a, "backup", 1).expect("search").len(), 1);
    chat.close().expect("close");
}

#[test]
fn a_session_name_finds_the_same_session_after_a_reopen() {
    let dir = TempDir::new();
    let chat = ChatHistory::open(dir.path()).expect("open");
    let x = chat.session("x").expect("x");
    assert_eq!(chat.session("x").expect("again"), x);
    chat.close().expect("close");
    let chat = ChatHistory::open(dir.path()).expect("reopen");
    assert_eq!(chat.session("x").expect("after reopen"), x);
    chat.close().expect("close");
}

#[test]
fn the_demo_runs_to_the_end() {
    let _turn = take_turn();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_chat-history")).output().expect("run the demo");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "demo failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("page 3:"), "{stdout}");
    assert!(stdout.contains("sessions keep their turns apart: true"), "{stdout}");
}
