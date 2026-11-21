//! End to end: the real server keeps a backup of its databases in a store, and `restore` turns the
//! store back into the units the server held.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use common::{Reply, Server, TempDir};
use kerno::{Db, UnitFilter};
use kerno_backup::{FsStore, Target, catalog, restore};
use kerno_server::auth::hash_hex;
use serde_json::{Value, json};

const ACME: &str = "acme-0000000000000000000000000000000001";
const GLOBEX: &str = "globex-00000000000000000000000000000001";

/// The tests of this file take turns. Most of them start a server process, and a child holds a
/// copy of every file its parent has open until it execs: if another test had a restored database
/// open at that moment, its `LOCK` stays held by the child after the test closes it, and opening
/// the directory (`restored`, `notes_in`) fails with `Locked`.
static TURNS: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    TURNS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A `file://` URL for `path`: `file:///srv/x` on Unix, `file:///C:/x` on Windows (issue #4).
fn url_of(path: &Path) -> String {
    let p = path.display().to_string().replace('\\', "/");
    if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{p}") }
}

fn note(server: &Server, headers: &[(&str, &str)], text: &str) {
    let body = json!({ "type": "note", "payload": { "text": text } });
    let reply = server.call("POST", "/v1/units", headers, Some(&body));
    assert_eq!(reply.status, 201, "{}", String::from_utf8_lossy(&reply.body));
}

fn metrics(server: &Server) -> String {
    String::from_utf8(server.get("/metrics").body).unwrap()
}

fn metric(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.parse().ok())
}

/// Waits until the server's metrics say `name` is at least `at_least`.
fn wait_for(server: &Server, name: &str, at_least: u64) {
    let until = Instant::now() + Duration::from_secs(20);
    while metric(&metrics(server), name).unwrap_or(0) < at_least {
        assert!(Instant::now() < until, "{name} never reached {at_least}:\n{}", metrics(server));
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The texts of the notes in a restored database, after checking it is sound.
fn notes_in(dir: &Path) -> BTreeSet<String> {
    let db = Db::open(dir).unwrap();
    assert!(db.unit_db().verify_units().unwrap().is_clean());
    let filter = UnitFilter { unit_type: Some("note".into()), ..UnitFilter::default() };
    let notes = db.query(&filter).unwrap();
    let texts = notes.iter().map(|u| String::from_utf8(u.payload.clone().unwrap()).unwrap()).collect();
    db.close().unwrap();
    texts
}

fn set(texts: &[&str]) -> BTreeSet<String> {
    texts.iter().map(|t| (*t).to_owned()).collect()
}

fn restored(store: &Path, prefix: &str, into: &Path) -> BTreeSet<String> {
    let url = url_of(store);
    let store = kerno_backup::open_store(&url).unwrap();
    let store = kerno_backup::Prefixed::new(store, prefix);
    restore(&store, into, Target::Latest).unwrap();
    notes_in(into)
}

fn log_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn a_server_ships_while_it_runs_and_a_last_time_when_it_stops() {
    let _turn = take_turn();
    let t = TempDir::new();
    let store = t.0.join("store");
    let log = t.0.join("server.log");
    let url = url_of(&store);
    let db = t.db();
    let server =
        Server::launch(&["--db", db.to_str().unwrap(), "--backup", &url, "--backup-interval", "1"], Some(&log));
    // The first tick takes the base, and a later one ships what is written after it.
    wait_for(&server, "kerno_backup_bases_total", 1);
    note(&server, &[], "one");
    note(&server, &[], "two");
    wait_for(&server, "kerno_backup_shipped_commits_total", 2);
    let text = metrics(&server);
    assert!(metric(&text, "kerno_backup_segments_total").unwrap() >= 1, "{text}");
    assert_eq!(metric(&text, "kerno_backup_errors_total"), Some(0), "{text}");
    assert!(text.contains("# TYPE kerno_backup_shipped_commits_total counter"), "{text}");

    // A base backup on demand.
    let reply = server.post("/v1/backup", &json!({}));
    assert_eq!(reply.status, 201, "{}", String::from_utf8_lossy(&reply.body));
    let base = reply.json();
    assert!(base["seq"].as_u64().unwrap() >= 2 && base["entries"].as_u64().unwrap() > 0, "{base}");
    assert_eq!(base["parts"], 1);
    let reply = server.get("/v1/backup");
    assert_eq!(reply.status, 405);
    assert_eq!(reply.header("allow"), Some("POST"));

    // The last write is stopped right after: only the shipment at shutdown can have it.
    let before = metric(&metrics(&server), "kerno_backup_shipped_commits_total").unwrap();
    note(&server, &[], "last");
    assert!(server.stop().success());
    let cat = catalog(&FsStore::new(&store)).unwrap();
    assert!(cat.bases.len() >= 2 && !cat.segments.is_empty());
    assert!(cat.segments.last().unwrap().last > before, "the final shipment is in the store");

    let copy = t.0.join("copy");
    assert_eq!(restored(&store, "", &copy), set(&["one", "two", "last"]));
    // The log says what the backups did, and carries no credential.
    let lines = log_lines(&log);
    assert!(lines.iter().any(|l| l["fields"]["message"] == "backup" && l["fields"]["base_commit"].is_number()));
}

#[test]
fn a_clean_stop_ships_the_commits_no_tick_has_seen() {
    let _turn = take_turn();
    let t = TempDir::new();
    let store = t.0.join("store");
    let url = url_of(&store);
    let db = t.db();
    // One tick only, at the start: the shipment at shutdown is the only way "last" gets out.
    let server = Server::launch(&["--db", db.to_str().unwrap(), "--backup", &url, "--backup-interval", "3600"], None);
    wait_for(&server, "kerno_backup_bases_total", 1);
    note(&server, &[], "last");
    assert_eq!(metric(&metrics(&server), "kerno_backup_shipped_commits_total"), Some(0));
    assert!(server.stop().success());
    assert_eq!(restored(&store, "", &t.0.join("copy")), set(&["last"]));
}

#[test]
fn a_server_that_died_takes_a_new_base_when_it_starts_again() {
    let _turn = take_turn();
    let t = TempDir::new();
    let store = t.0.join("store");
    let url = url_of(&store);
    let db = t.db();
    // One shipment only, at the start: nothing after it is shipped before the process is killed.
    let args = ["--db", db.to_str().unwrap(), "--backup", &url, "--backup-interval", "3600"];
    let server = Server::launch(&args, None);
    wait_for(&server, "kerno_backup_bases_total", 1);
    note(&server, &[], "before the crash");
    let status = std::process::Command::new("kill").args(["-KILL", &server.id().to_string()]).status().unwrap();
    assert!(status.success());
    drop(server);

    let again = Server::launch(&args, None);
    // The engine replayed the commit and deleted its log, so only a new base has it.
    wait_for(&again, "kerno_backup_bases_total", 1);
    note(&again, &[], "after");
    assert!(again.stop().success());
    assert_eq!(restored(&store, "", &t.0.join("copy")), set(&["before the crash", "after"]));
}

#[test]
fn each_tenant_has_its_own_backups_and_the_route_needs_its_token() {
    let _turn = take_turn();
    let t = TempDir::new();
    let tokens = t.0.join("tokens");
    std::fs::write(&tokens, format!("{} acme\n{} globex\n", hash_hex(ACME), hash_hex(GLOBEX))).unwrap();
    let (root, store) = (t.0.join("tenants"), t.0.join("store"));
    let url = url_of(&store);
    let server = Server::launch(
        &[
            "--data-dir",
            root.to_str().unwrap(),
            "--tokens",
            tokens.to_str().unwrap(),
            "--backup",
            &url,
            "--backup-interval",
            "1",
        ],
        None,
    );
    let (acme, globex) =
        ([("Authorization", &*format!("Bearer {ACME}"))], [("Authorization", &*format!("Bearer {GLOBEX}"))]);
    note(&server, &acme, "acme's");
    note(&server, &globex, "globex's");
    // Two tenants, a base each at the start. Wait for both before asking for one: a base taken on
    // demand ahead of a tenant's first tick is that tenant's only base (the tick finds it and takes
    // none), and the server is up and answering within a millisecond of starting that tick.
    wait_for(&server, "kerno_backup_bases_total", 2);
    // No token, no backup; the token of a tenant backs up that tenant.
    let no_token: Reply = server.call("POST", "/v1/backup", &[], Some(&json!({})));
    assert_eq!(no_token.status, 401);
    let base = server.call("POST", "/v1/backup", &acme, Some(&json!({})));
    assert_eq!(base.status, 201, "{}", String::from_utf8_lossy(&base.body));
    // The counter sums the two start bases and the one on demand.
    wait_for(&server, "kerno_backup_bases_total", 3);
    assert!(server.stop().success());

    assert_eq!(restored(&store, "acme", &t.0.join("acme-copy")), set(&["acme's"]));
    assert_eq!(restored(&store, "globex", &t.0.join("globex-copy")), set(&["globex's"]));
    let top = FsStore::new(&store);
    let keys = kerno_backup::ObjectStore::list(&top, "").unwrap();
    assert!(keys.iter().all(|k| k.starts_with("acme/") || k.starts_with("globex/")), "{keys:?}");
}

#[test]
fn without_backups_the_route_says_so_and_the_metrics_have_no_backup_family() {
    let _turn = take_turn();
    let t = TempDir::new();
    let server = Server::start(&t.db());
    let reply = server.post("/v1/backup", &json!({}));
    assert_eq!(reply.status, 404);
    let message = reply.json()["error"]["message"].as_str().unwrap().to_owned();
    assert_eq!(message, "backups are not configured: start the server with --backup <url>");
    assert!(!metrics(&server).contains("kerno_backup"));
}

#[test]
fn a_bad_backup_flag_or_location_stops_the_server_before_it_listens() {
    let _turn = take_turn();
    let t = TempDir::new();
    let db = t.db();
    let run = |extra: &[&str]| {
        let mut args = vec!["--db", db.to_str().unwrap()];
        args.extend_from_slice(extra);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_kerno-server"))
            .args(&args)
            .env_remove("KERNO_SERVER_DB")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        (out.status.code(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
    };
    let (code, out, err) = run(&["--backup-interval", "5"]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.starts_with("kerno-server: --backup-interval needs --backup <url>"), "{err}");
    let (code, out, err) = run(&["--backup", "ftp://nowhere"]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.starts_with("kerno-server: cannot use the backup location ftp://nowhere: "), "{err}");
    assert!(!db.exists(), "nothing is opened before the location is checked");
}
