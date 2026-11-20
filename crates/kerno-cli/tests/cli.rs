//! End to end: every test runs the real `kerno` binary on a temporary database
//! that the test made through the public API of `kerno-units`.

// The no-panic rule is for the tool; a test panics to fail.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use kerno_core::WriteBatch;
use kerno_core::contract::Engine;
use kerno_core::vfs::StdVfs;
use kerno_storage::{Config, KernoDb};
use kerno_units::{
    ChangeMeta, CounterIds, Document, FieldDef, FieldType, FieldValue, FixedClock, Key, NewUnit, RelationId,
    RelationProvenance, Timestamp, UnitConfig, UnitDb, UnitId, UnitUpdate,
};
use serde_json::Value;

/// A fresh directory that is removed when the test ends, pass or fail.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir =
            std::env::temp_dir().join(format!("kerno-cli-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn db(&self) -> PathBuf {
        self.0.join("db")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn kerno_once(args: &[&str]) -> Run {
    let o = Command::new(env!("CARGO_BIN_EXE_kerno")).args(args).output().unwrap();
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8(o.stdout).unwrap(),
        err: String::from_utf8(o.stderr).unwrap(),
    }
}

/// Runs `kerno`, trying again while a lock is held that nobody holds on purpose. The tests share one
/// process: a child that another test's thread forked inherits that test's lock file until its `exec`,
/// so a database can look locked for a moment after its handle was closed.
fn kerno(args: &[&str]) -> Run {
    for _ in 0..200 {
        let r = kerno_once(args);
        if r.code != 1 || !r.err.contains("locked by another handle") {
            return r;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    kerno_once(args)
}

fn run_in(dir: &Path, args: &[&str]) -> Run {
    let mut all = vec![dir.to_str().unwrap()];
    all.extend_from_slice(args);
    kerno(&all)
}

/// A successful command's JSON.
fn json_of(dir: &Path, args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let r = run_in(dir, &all);
    assert_eq!(r.code, 0, "{args:?}: {}", r.err);
    serde_json::from_str(&r.out).unwrap_or_else(|e| panic!("{args:?} did not print JSON ({e}): {}", r.out))
}

/// The JSON of a command that exits 1 (a `verify` with findings).
fn json_of_failing(dir: &Path, args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let r = run_in(dir, &all);
    assert_eq!(r.code, 1, "{args:?}: {} {}", r.out, r.err);
    serde_json::from_str(&r.out).unwrap()
}

struct Fixture {
    note_a: UnitId,
    public_a: String,
    note_b: UnitId,
    task_c: UnitId,
    cites: RelationId,
}

/// Opens the engine, waiting out a lock that a handle closed a moment ago has not released yet.
fn open_engine(dir: &Path, cfg: &Config) -> KernoDb {
    for _ in 0..500 {
        match KernoDb::open(StdVfs::shared(), dir, cfg) {
            Err(kerno_core::Error::Locked) => std::thread::sleep(std::time::Duration::from_millis(10)),
            other => return other.unwrap(),
        }
    }
    panic!("{} stays locked", dir.display());
}

/// Closes a unit database and waits until its lock is free, so that the next process can open it.
fn close_and_wait(db: UnitDb<KernoDb>, dir: &Path) {
    db.close().unwrap();
    wait_unlocked(dir);
}

fn wait_unlocked(dir: &Path) {
    let cfg = Config { access: kerno_storage::AccessMode::ReadOnly, ..Config::default() };
    open_engine(dir, &cfg).close().unwrap();
}

fn open_units(dir: &Path, write_buffer: Option<usize>) -> UnitDb<KernoDb> {
    let mut cfg = Config::default();
    if let Some(n) = write_buffer {
        cfg.write_buffer_bytes = n;
    }
    let engine = open_engine(dir, &cfg);
    let clock = Arc::new(FixedClock::stepping(Timestamp(1_000_000), 1000));
    UnitDb::from_engine(engine, &UnitConfig::with_sources(clock, Arc::new(CounterIds::new(1)))).unwrap()
}

fn new_unit(unit_type: &str, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: unit_type.into(), schema_version: 1, metadata: b"meta".to_vec(), payload: payload.to_vec() }
}

/// Five units (A and B and an archived and a deleted "note", C a "task"), A with two
/// revisions, two relations (A -cites-> B, C -blocks-> A), the primary index built.
fn build(dir: &Path, index: bool) -> Fixture {
    let db = open_units(dir, None);
    let meta =
        || ChangeMeta { reason: Some("fixture".into()), actor: Some(b"tester".to_vec()), ..ChangeMeta::default() };
    let mut cs = db.begin_changeset(meta()).unwrap();
    let (note_a, token) = cs.create_public(new_unit("note", b"alpha")).unwrap();
    let note_b = cs.create(new_unit("note", &[0xff, 0xfe, 0x00, 0x80])).unwrap();
    let task_c = cs.create(new_unit("task", b"do it")).unwrap();
    let archived = cs.create(new_unit("note", b"old")).unwrap();
    let deleted = cs.create(new_unit("note", b"gone")).unwrap();
    cs.commit().unwrap();

    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(note_a, UnitUpdate { payload: b"alpha two".to_vec(), schema_version: None, metadata: None }, None)
        .unwrap();
    cs.archive(archived, None).unwrap();
    cs.delete(deleted, None).unwrap();
    let cites = cs.create_relation(note_a, note_b, "cites", RelationProvenance::Declared).unwrap();
    cs.create_relation(task_c, note_a, "blocks", RelationProvenance::Imported { source: b"jira".to_vec() }).unwrap();
    cs.commit().unwrap();
    if index {
        db.rebuild_index().unwrap();
    }
    close_and_wait(db, dir);
    Fixture { note_a, public_a: token.to_string(), note_b, task_c, cites }
}

/// 300 units of 300 bytes on a 4 KiB write buffer, compacted: the files include tables, and the
/// units take more than one page of `stats`.
fn build_with_tables(dir: &Path, reclaim: bool) {
    let db = open_units(dir, Some(4096));
    for chunk in 0..30 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        for i in 0..10 {
            cs.create(new_unit("blob", format!("{chunk}-{i}-{}", "x".repeat(300)).as_bytes())).unwrap();
        }
        cs.commit().unwrap();
    }
    if reclaim {
        db.reclaim().unwrap();
    }
    close_and_wait(db, dir);
    assert!(files_of(dir).keys().any(|n| n.ends_with(".sst")), "the fixture needs a table: {:?}", files_of(dir).keys());
}

/// Every file of a directory by name, with its bytes.
fn files_of(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())
        })
        .collect()
}

// ---------------------------------------------------------------- usage

#[test]
fn help_prints_every_command_and_the_two_warnings_without_opening_anything() {
    let t = TempDir::new();
    let missing = t.path("missing");
    for r in [kerno(&["--help"]), kerno(&[missing.to_str().unwrap(), "--help"]), run_in(&missing, &["stats", "--help"])]
    {
        assert_eq!(r.code, 0, "{}", r.err);
        assert!(r.out.contains("kerno <dir> "), "{}", r.out);
    }
    let all = kerno(&["--help"]).out;
    for command in [
        "verify",
        "stats",
        "get",
        "history",
        "relations",
        "query",
        "export",
        "import",
        "backup",
        "ship",
        "restore",
        "backups",
        "reclaim",
    ] {
        assert!(
            all.contains(&format!("kerno <dir> {command}")) || all.contains(&format!("kerno {command}")),
            "{command}"
        );
    }
    let lower = all.to_lowercase();
    assert!(lower.contains("frozen at format_version 1") && !lower.contains("draft"), "{all}");
    assert!(lower.contains("not a secure erase"), "{all}");
    // The help of one command is that command's.
    let reclaim = run_in(&missing, &["reclaim", "--help"]).out.to_lowercase();
    assert!(reclaim.contains("not a secure erase") && !reclaim.contains("draft"), "{reclaim}");
    let export = run_in(&missing, &["export", "--help"]).out.to_lowercase();
    assert!(
        export.contains("frozen at format_version 1")
            && export.contains("export-v1.md")
            && !export.contains("draft")
            && !export.contains("secure erase"),
        "{export}"
    );
    let import = run_in(&missing, &["import", "--help"]).out.to_lowercase();
    assert!(import.contains("export-v1.md") && !import.contains("draft") && !import.contains("not frozen"), "{import}");
    assert!(!missing.exists(), "--help must not create the database directory");
}

#[test]
fn a_wrong_command_line_is_one_line_on_stderr_and_exit_2() {
    let t = TempDir::new();
    build(&t.db(), true);
    let db = t.db();
    let d = db.to_str().unwrap();
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec![d],
        vec![d, "frobnicate"],
        vec![d, "stats", "--bogus"],
        vec![d, "stats", "extra"],
        vec![d, "get"],
        vec![d, "get", "not-an-id"],
        vec![d, "relations", "00000000000000000000000000000001", "--dir", "sideways"],
        vec![d, "relations", "00000000000000000000000000000001", "--dir"],
        vec![d, "query"],
        vec![d, "query", "--type", "note", "--limit", "0"],
        vec![d, "query", "--type", "note", "--limit", "many"],
        vec![d, "verify", "--deep", "--deep"],
        vec![d, "import", "only-one-argument"],
        vec!["--frobnicate"],
    ];
    for args in cases {
        let r = kerno(&args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        assert!(r.out.is_empty(), "{args:?}: {}", r.out);
        assert!(r.err.starts_with("kerno: ") && r.err.trim_end().lines().count() == 1, "{args:?}: {:?}", r.err);
    }
}

#[test]
fn a_wrong_argument_count_says_how_many_were_expected_and_the_usage() {
    let t = TempDir::new();
    build(&t.db(), true);
    let db = t.db();
    let d = db.to_str().unwrap();
    let err = |args: &[&str]| {
        let r = kerno(args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        r.err.trim_end().to_owned()
    };
    // A command that takes an exact number says that number.
    assert_eq!(err(&[d, "get"]), "kerno: get: expected 1 argument(s), got 0 (usage: get <unit-id> [--json])");
    assert_eq!(err(&[d, "stats", "extra"]), "kerno: stats: expected 0 argument(s), got 1 (usage: stats [--json])");
    assert_eq!(
        err(&[d, "import", "only-one"]),
        "kerno: import: expected 2 argument(s), got 1 (usage: import <file> <new-dir>)"
    );
    // One that takes an optional argument says the range.
    let usage = "(<query> | --type T [--limit N]) [--explain] [--json]";
    assert_eq!(
        err(&[d, "query", "FIND a", "FIND b"]),
        format!("kerno: query: expected 0 to 1 argument(s), got 2 (usage: query {usage})")
    );
    assert_eq!(
        err(&[d, "query", "a", "b", "c"]),
        format!("kerno: query: expected 0 to 1 argument(s), got 3 (usage: query {usage})")
    );
    // The forms of `query` that cannot go together, and the one with nothing to run.
    let both =
        "kerno: query: give a query text or --type, not both (--limit goes with --type; a query has its own LIMIT)";
    assert_eq!(err(&[d, "query", "FIND user", "--type", "user"]), both);
    assert_eq!(err(&[d, "query", "FIND user", "--limit", "1"]), both);
    assert_eq!(err(&[d, "query", "--limit", "1"]), "kerno: query: give a query text or --type");
    assert_eq!(err(&[d, "query"]), "kerno: query: give a query text or --type");
    // One text passes the count check, with flags or without (no field is read, so no schema is needed).
    for flags in [&[][..], &["--explain", "--json"]] {
        let mut args = vec![d, "query", "FIND note"];
        args.extend_from_slice(flags);
        assert_eq!(kerno(&args).code, 0, "{flags:?}");
    }
}

#[test]
fn an_argument_that_is_not_unicode_is_a_usage_error_not_a_panic() {
    use std::os::unix::ffi::OsStrExt;
    let bad = std::ffi::OsStr::from_bytes(b"/tmp/\xff");
    let o = Command::new(env!("CARGO_BIN_EXE_kerno")).arg(bad).arg("stats").output().unwrap();
    let err = String::from_utf8(o.stderr).unwrap();
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(o.stdout.is_empty());
    assert!(err.starts_with("kerno: ") && err.trim_end().lines().count() == 1, "{err:?}");
}

#[test]
fn a_failure_is_exit_1_and_creates_nothing() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    // No database: every command fails and none makes one.
    let missing = t.path("missing");
    for args in [
        &["stats"][..],
        &["verify"],
        &["query", "--type", "note"],
        &["get", "00000000000000000000000000000001"],
        &["reclaim"],
    ] {
        let r = run_in(&missing, args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.err);
        assert!(r.err.starts_with("kerno: ") && r.err.trim_end().lines().count() == 1, "{args:?}: {:?}", r.err);
        assert!(!missing.exists(), "{args:?} created the directory");
    }
    // A well-formed id that names nothing, and a public id that names nothing.
    for id in ["ffffffffffffffffffffffffffffffff".to_owned(), format!("k1_{}", "a".repeat(26))] {
        let r = run_in(&t.db(), &["get", &id]);
        assert_eq!(r.code, 1, "{id}: {} {}", r.out, r.err);
        assert!(r.err.contains("no unit"), "{}", r.err);
    }
    // An export never replaces a file.
    let file = t.path("out.kexp");
    std::fs::write(&file, b"keep me").unwrap();
    let r = run_in(&t.db(), &["export", file.to_str().unwrap()]);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(std::fs::read(&file).unwrap(), b"keep me");
    let _ = fx;
}

// --------------------------------------------------------------- verify

#[test]
fn verify_of_a_sound_database_is_exit_0_and_says_which_checks_ran() {
    let t = TempDir::new();
    build(&t.db(), true);
    let shallow = run_in(&t.db(), &["verify"]);
    assert_eq!(shallow.code, 0, "{} {}", shallow.out, shallow.err);
    assert!(
        shallow.out.contains("engine (open-time checks") && shallow.out.contains("units: no findings"),
        "{}",
        shallow.out
    );
    let deep = run_in(&t.db(), &["verify", "--deep"]);
    assert_eq!(deep.code, 0, "{} {}", deep.out, deep.err);
    assert!(deep.out.contains("engine (deep): no findings"), "{}", deep.out);
    // verify_units really ran: it counted the five units and two relations.
    let json = json_of(&t.db(), &["verify", "--deep"]);
    assert_eq!(json["ok"], true);
    assert_eq!(json["deep"], true);
    assert_eq!(json["units"]["counts"]["units"], 5);
    assert_eq!(json["units"]["counts"]["relations"], 2);
    assert_eq!(json_of(&t.db(), &["verify"])["deep"], false);
}

#[test]
fn verify_reports_a_lost_adjacency_entry_of_the_units_layer_and_exits_1() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    // Lose the outgoing adjacency entry of A -cites-> B, below the layer, through the engine's public API.
    {
        let engine = open_engine(&t.db(), &Config::default());
        let mut batch = WriteBatch::new();
        batch.delete(
            Key::RelationOut { unit: fx.note_a, relation_type: "cites".into(), edge: fx.cites }.encode().unwrap(),
        );
        engine.commit(&batch).unwrap();
        Engine::close(engine).unwrap();
    }
    wait_unlocked(&t.db());
    let r = run_in(&t.db(), &["verify"]);
    assert_eq!(r.code, 1, "{} {}", r.out, r.err);
    assert!(
        r.out.contains("engine (open-time checks") && r.out.contains(": no findings"),
        "the files are sound: {}",
        r.out
    );
    assert!(r.out.contains("units: ") && r.out.contains("finding(s)"), "{}", r.out);
    assert!(r.out.contains(&fx.cites.to_string()), "the finding names the relation: {}", r.out);
    let json = json_of_failing(&t.db(), &["verify"]);
    assert_eq!(json["ok"], false);
    assert!(!json["units"]["findings"].as_array().unwrap().is_empty());
    assert!(json["engine"]["findings"].as_array().unwrap().is_empty());
}

#[test]
fn verify_deep_reports_a_damaged_table_block_and_exits_1() {
    let t = TempDir::new();
    build_with_tables(&t.db(), true);
    assert_eq!(run_in(&t.db(), &["verify", "--deep"]).code, 0);
    // The last table holds the highest keys: not the layer mark, which an open reads.
    let table = files_of(&t.db()).into_keys().rfind(|n| n.ends_with(".sst")).unwrap();
    let path = t.db().join(&table);
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.len() / 3;
    bytes[at] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();

    let deep = run_in(&t.db(), &["verify", "--deep"]);
    assert_eq!(deep.code, 1, "{} {}", deep.out, deep.err);
    assert!(deep.out.contains("engine (deep): ") && !deep.out.contains("engine (deep): no findings"), "{}", deep.out);
    assert!(deep.out.contains(&format!("SST-R1 {table}")), "the engine's finding names rule and file: {}", deep.out);
    assert!(!json_of_failing(&t.db(), &["verify", "--deep"])["engine"]["findings"].as_array().unwrap().is_empty());
    // Without --deep the engine reads no block, so its own check finds nothing; the layer's read of the
    // same block still fails, which is why the exit is 1 too.
    let shallow = run_in(&t.db(), &["verify"]);
    assert_eq!(shallow.code, 1, "{} {}", shallow.out, shallow.err);
    assert!(
        shallow.out.contains("engine (open-time checks; --deep reads every block): no findings"),
        "{}",
        shallow.out
    );
    assert!(
        shallow.out.contains("units: cannot be checked") && shallow.out.contains("block CRC mismatch"),
        "{}",
        shallow.out
    );
    assert!(json_of_failing(&t.db(), &["verify"])["engine"]["findings"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------- stats

#[test]
fn stats_counts_units_and_relations_by_type_and_names_the_versions_and_files() {
    let t = TempDir::new();
    build(&t.db(), true);
    let json = json_of(&t.db(), &["stats"]);
    assert_eq!(json["units"]["live"], 4);
    assert_eq!(json["units"]["by_type"]["note"], serde_json::json!({ "active": 2, "archived": 1, "deleted": 1 }));
    assert_eq!(json["units"]["by_type"]["task"], serde_json::json!({ "active": 1, "archived": 0, "deleted": 0 }));
    assert_eq!(json["relations"]["total"], 2);
    assert_eq!(json["relations"]["by_type"], serde_json::json!({ "blocks": 1, "cites": 1 }));
    assert_eq!(json["index"]["state"], "Ready");
    // The layer the fixture was written at (units and relations: live-unit counts),
    // not the newest this build can write.
    assert_eq!(json["layer_version"], kerno_units::codec::LAYER_VERSION_TYPE_COUNTS);
    assert_eq!(json["format_versions"], serde_json::json!([1]));
    assert_eq!(json["last_seq"], json["index"]["watermark"]);
    let on_disk = files_of(&t.db());
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), on_disk.len());
    for f in files {
        assert_eq!(f["bytes"].as_u64().unwrap(), on_disk[f["name"].as_str().unwrap()].len() as u64);
    }
    assert_eq!(json["total_bytes"].as_u64().unwrap(), on_disk.values().map(|b| b.len() as u64).sum::<u64>());

    let text = run_in(&t.db(), &["stats"]).out;
    for needle in [
        "units         4 live (1 deleted)",
        "note: 2 active, 1 archived, 1 deleted",
        "blocks: 1",
        "cites: 1",
        "primary Ready",
        "MANIFEST",
        &format!("layer version {}", kerno_units::codec::LAYER_VERSION_TYPE_COUNTS),
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
}

#[test]
fn stats_shows_an_index_that_was_never_built_and_an_empty_database() {
    let t = TempDir::new();
    build(&t.db(), false);
    assert_ne!(json_of(&t.db(), &["stats"])["index"]["state"], "Ready");
    // A database with no commit has no layer mark and no units.
    close_and_wait(open_units(&t.path("empty"), None), &t.path("empty"));
    let json = json_of(&t.path("empty"), &["stats"]);
    assert_eq!(json["layer_version"], Value::Null);
    assert_eq!(json["units"]["live"], 0);
    assert_eq!(json["relations"]["total"], 0);
}

// ----------------------------------------------- get, history, relations

#[test]
fn get_prints_text_as_text_and_binary_as_hex_with_its_length() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    let text = run_in(&t.db(), &["get", &fx.note_a.to_string()]);
    assert_eq!(text.code, 0, "{}", text.err);
    for needle in [
        "type       note",
        "revision   2 (oldest retained 1)",
        "payload    9 bytes, text \"alpha two\"",
        "metadata   4 bytes, text \"meta\"",
        &fx.public_a,
    ] {
        assert!(text.out.contains(needle), "{needle}\n{}", text.out);
    }
    let binary = run_in(&t.db(), &["get", &fx.note_b.to_string()]).out;
    assert!(binary.contains("payload    4 bytes, hex fffe0080"), "{binary}");
    let json = json_of(&t.db(), &["get", &fx.note_b.to_string()]);
    assert_eq!(json["payload"], serde_json::json!({ "len": 4, "encoding": "hex", "hex": "fffe0080" }));
    assert_eq!(json["lifecycle"], "Active");
    assert_eq!(json["revision"], 1);
    // The public id names the same unit.
    let by_token = json_of(&t.db(), &["get", &fx.public_a]);
    assert_eq!(by_token["id"], fx.note_a.to_string());
    assert_eq!(by_token["public_id"], fx.public_a);
    assert_eq!(by_token["payload"]["text"], "alpha two");
}

#[test]
fn history_pages_through_more_revisions_than_one_page_holds() {
    let t = TempDir::new();
    let db = open_units(&t.db(), None);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.create(new_unit("note", b"v1")).unwrap();
    cs.commit().unwrap();
    // One page is 1000 revisions: 1005 make a full page and a short one.
    for n in 2..=1005 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(id, UnitUpdate { payload: format!("v{n}").into_bytes(), schema_version: None, metadata: None }, None)
            .unwrap();
        cs.commit().unwrap();
    }
    close_and_wait(db, &t.db());
    let json = json_of(&t.db(), &["history", &id.to_string()]);
    let revs: Vec<u64> =
        json["revisions"].as_array().unwrap().iter().map(|r| r["revision"].as_u64().unwrap()).collect();
    assert_eq!(revs, (1..=1005).collect::<Vec<u64>>());
}

#[test]
fn history_lists_the_retained_revisions_oldest_first_with_their_payloads() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    let json = json_of(&t.db(), &["history", &fx.note_a.to_string()]);
    let revs = json["revisions"].as_array().unwrap();
    assert_eq!(revs.len(), 2);
    assert_eq!((revs[0]["revision"].as_u64(), revs[0]["kind"].as_str()), (Some(1), Some("Create")));
    assert_eq!((revs[1]["revision"].as_u64(), revs[1]["kind"].as_str()), (Some(2), Some("Update")));
    assert_eq!(revs[0]["payload"]["text"], "alpha");
    assert_eq!(revs[1]["payload"]["text"], "alpha two");
    assert_eq!(revs[1]["actor"], "tester");
    let text = run_in(&t.db(), &["history", &fx.note_a.to_string()]).out;
    assert!(
        text.contains("2 retained revision(s)")
            && text.contains("revision 1 Create")
            && text.contains("revision 2 Update"),
        "{text}"
    );
    assert!(text.find("revision 1").unwrap() < text.find("revision 2").unwrap());
    let none = run_in(&t.db(), &["history", "00000000000000000000000000000009"]);
    assert_eq!(none.code, 1, "{}", none.out);
}

fn relation_rows(json: &Value) -> Vec<(String, String)> {
    json["relations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["direction"].as_str().unwrap().to_owned(), r["type"].as_str().unwrap().to_owned()))
        .collect()
}

#[test]
fn relations_without_dir_shows_both_directions_and_the_options_narrow_them() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    let a = fx.note_a.to_string();
    // A has one edge out (cites B) and one in (C blocks A).
    let both = json_of(&t.db(), &["relations", &a]);
    assert_eq!(relation_rows(&both), [("out".into(), "cites".into()), ("in".into(), "blocks".into())]);
    assert_eq!(relation_rows(&json_of(&t.db(), &["relations", &a, "--dir", "out"])), [("out".into(), "cites".into())]);
    assert_eq!(relation_rows(&json_of(&t.db(), &["relations", &a, "--dir", "in"])), [("in".into(), "blocks".into())]);
    assert_eq!(
        relation_rows(&json_of(&t.db(), &["relations", &a, "--type", "blocks"])),
        [("in".into(), "blocks".into())]
    );
    assert!(relation_rows(&json_of(&t.db(), &["relations", &a, "--dir", "out", "--type", "blocks"])).is_empty());
    // Both ends and provenance are in the answer.
    let out = &json_of(&t.db(), &["relations", &a, "--dir", "out"])["relations"][0];
    assert_eq!((out["from"].as_str(), out["to"].as_str()), (Some(a.as_str()), Some(fx.note_b.to_string().as_str())));
    assert_eq!(out["provenance"], "declared");
    let inn = &json_of(&t.db(), &["relations", &a, "--dir", "in"])["relations"][0];
    assert_eq!(inn["from"], fx.task_c.to_string());
    assert_eq!(inn["provenance"], "imported from jira");
    let text = run_in(&t.db(), &["relations", &a]).out;
    assert!(text.contains("2 relation(s)") && text.contains("cites") && text.contains("blocks"), "{text}");
}

// ---------------------------------------------------------------- query

#[test]
fn query_lists_the_units_of_a_type_limits_them_and_explains_the_plan() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    let json = json_of(&t.db(), &["query", "--type", "note"]);
    assert_eq!(json["total"], 2, "the archived and the deleted note are not active");
    let ids: Vec<&str> = json["units"].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap()).collect();
    let mut expect = [fx.note_a.to_string(), fx.note_b.to_string()];
    expect.sort();
    assert_eq!(ids, expect);
    assert_eq!(json_of(&t.db(), &["query", "--type", "task"])["units"][0]["id"], fx.task_c.to_string());
    assert_eq!(json_of(&t.db(), &["query", "--type", "nothing"])["total"], 0);

    let limited = json_of(&t.db(), &["query", "--type", "note", "--limit", "1"]);
    assert_eq!((limited["total"].as_u64(), limited["units"].as_array().unwrap().len()), (Some(2), 1));
    assert_eq!(limited["units"][0]["id"], expect[0]);
    let text = run_in(&t.db(), &["query", "--type", "note", "--limit", "1"]).out;
    assert!(
        text.contains("2 unit(s) of type \"note\", showing 1")
            && text.contains(&expect[0])
            && !text.contains(&expect[1]),
        "{text}"
    );

    // Without --explain there is no plan; with it, the planner's: the ready type index is chosen.
    assert!(json.get("plan").is_none());
    let plan = &json_of(&t.db(), &["query", "--type", "note", "--explain"])["plan"];
    assert_eq!(plan["index"]["state"], "Ready");
    assert!(plan["chosen"].as_str().unwrap().starts_with("Index"), "{plan}");
    assert_eq!(plan["candidates"].as_array().unwrap().len(), 3);
    let explained = run_in(&t.db(), &["query", "--type", "note", "--explain"]).out;
    assert!(
        explained.contains("plan units/blake3-keyed/v1/") && explained.contains("candidate UnitsReference cost"),
        "{explained}"
    );
}

#[test]
fn query_goes_through_the_planner_so_an_index_that_is_not_ready_is_not_used() {
    let t = TempDir::new();
    build(&t.db(), false);
    let json = json_of(&t.db(), &["query", "--type", "note", "--explain"]);
    assert_eq!(json["path"], "UnitsReference");
    assert_eq!(json["plan"]["chosen"], "UnitsReference");
    assert_eq!(json["total"], 2);
    let ready = TempDir::new();
    build(&ready.db(), true);
    assert!(json_of(&ready.db(), &["query", "--type", "note"])["path"].as_str().unwrap().starts_with("Index"));
}

// ------------------------------------------------- the query language

struct Lang {
    ada: UnitId,
    bob: UnitId,
    cy: UnitId,
    core: UnitId,
    ops: UnitId,
    note: UnitId,
}

/// Users and teams with schemas (email unique, age indexed), `member_of` relations, and a free-text
/// `note` with no schema that ada wrote.
fn build_lang(dir: &Path) -> Lang {
    let db = open_units(dir, None);
    let meta = ChangeMeta::default;
    db.define_schema(
        "user",
        vec![
            FieldDef::new("email", FieldType::Text).unique().required(),
            FieldDef::new("age", FieldType::Int).indexed(),
            FieldDef::new("vip", FieldType::Bool),
            FieldDef::new("score", FieldType::Float),
            FieldDef::new("blob", FieldType::Bytes),
        ],
        meta(),
    )
    .unwrap();
    db.define_schema("team", vec![FieldDef::new("name", FieldType::Text).unique().required()], meta()).unwrap();
    let user = |email: &str, age: i64| {
        let doc = Document::new().with("email", FieldValue::Text(email.into())).with("age", FieldValue::Int(age));
        // Ada has one field of every type.
        let doc = match email {
            "ada@x" => doc
                .with("vip", FieldValue::Bool(true))
                .with("score", FieldValue::Float(2.5))
                .with("blob", FieldValue::Bytes(vec![0xde, 0xad])),
            _ => doc,
        };
        new_unit("user", &doc.encode().unwrap())
    };
    let team =
        |name: &str| new_unit("team", &Document::new().with("name", FieldValue::Text(name.into())).encode().unwrap());
    let mut cs = db.begin_changeset(meta()).unwrap();
    let (ada, bob, cy) = (
        cs.create(user("ada@x", 36)).unwrap(),
        cs.create(user("bob@x", 41)).unwrap(),
        cs.create(user("cy@x", 29)).unwrap(),
    );
    let (core, ops) = (cs.create(team("core")).unwrap(), cs.create(team("ops")).unwrap());
    let note = cs.create(new_unit("note", b"hello")).unwrap();
    for (who, to) in [(ada, core), (bob, core), (cy, ops)] {
        cs.create_relation(who, to, "member_of", RelationProvenance::Declared).unwrap();
    }
    cs.create_relation(ada, note, "wrote", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    close_and_wait(db, dir);
    Lang { ada, bob, cy, core, ops, note }
}

/// The ids of each row of a `query --json` answer, by column.
fn rows_of(json: &Value) -> Vec<Vec<String>> {
    json["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap().to_owned()).collect())
        .collect()
}

#[test]
fn query_text_filters_orders_limits_and_pages_the_units_of_a_type() {
    let t = TempDir::new();
    let l = build_lang(&t.db());
    let q = |text: &str| json_of(&t.db(), &["query", text]);
    let one = |u: UnitId| vec![u.to_string()];
    assert_eq!(rows_of(&q("FIND user WHERE age >= 36 ORDER BY age DESC")), [one(l.bob), one(l.ada)]);
    assert_eq!(rows_of(&q("FIND user ORDER BY age")), [one(l.cy), one(l.ada), one(l.bob)]);
    assert_eq!(rows_of(&q("FIND user ORDER BY age LIMIT 1 OFFSET 1")), [one(l.ada)]);
    assert_eq!(
        rows_of(&q("FIND user WHERE email = \"cy@x\" OR age = 41 ORDER BY email DESC")),
        [one(l.cy), one(l.bob)]
    );
    let json = q("FIND user WHERE age > 36 ORDER BY age");
    assert_eq!((json["total"].as_u64(), json["columns"].clone()), (Some(1), serde_json::json!(["user"])));
    assert_eq!(json["rows"][0][0]["fields"], serde_json::json!({ "age": 41, "email": "bob@x" }));
    assert_eq!(json["query"], "FIND user AS user WHERE user.age > 36 ORDER BY user.age ASC");
    assert!(json.get("plan").is_none(), "no plan without --explain");
    // A field of every type, as JSON and as text.
    let ada = q("FIND user WHERE email = \"ada@x\"");
    assert_eq!(
        ada["rows"][0][0]["fields"],
        serde_json::json!({ "age": 36, "blob": { "hex": "dead" }, "email": "ada@x", "score": 2.5, "vip": true })
    );
    let line = run_in(&t.db(), &["query", "FIND user WHERE email = \"ada@x\""]).out;
    assert!(line.contains("{age=36, blob=x\"dead\", email=\"ada@x\", score=2.5, vip=true}"), "{line}");
    assert_eq!(rows_of(&q("FIND user WHERE age > 99")), Vec::<Vec<String>>::new());
    assert_eq!(q("FIND nothing")["total"], 0);

    // The text: a header with the way the first binding was read, then a line per row.
    let text = run_in(&t.db(), &["query", "FIND user WHERE age = 29"]);
    assert_eq!(text.code, 0, "{}", text.err);
    let lines: Vec<&str> = text.out.lines().collect();
    assert_eq!(lines.len(), 2, "{}", text.out);
    assert!(lines[0].starts_with("1 row(s) of (user); first binding read by FieldIndex(age)"), "{}", lines[0]);
    assert!(lines[1].starts_with(&format!("user {} user {{age=29, email=\"cy@x\"}}", l.cy)), "{}", lines[1]);
    let scan = run_in(&t.db(), &["query", "FIND user"]).out;
    assert!(scan.contains("first binding read by TypeScan through UnitsReference"), "{scan}");
}

#[test]
fn query_text_joins_unit_types_through_relations_in_both_directions() {
    let t = TempDir::new();
    let l = build_lang(&t.db());
    let q = |text: &str| json_of(&t.db(), &["query", text]);
    let json = q("FIND user AS u JOIN team AS t VIA member_of WHERE t.name = \"core\" ORDER BY u.age DESC");
    assert_eq!(
        rows_of(&json),
        [vec![l.bob.to_string(), l.core.to_string()], vec![l.ada.to_string(), l.core.to_string()]]
    );
    assert_eq!(json["columns"], serde_json::json!(["u", "t"]));
    assert_eq!(json["rows"][0][1]["fields"]["name"], "core");
    assert_eq!((json["rows"][0][0]["revision"].as_u64(), json["rows"][0][1]["type"].as_str()), (Some(1), Some("team")));
    // The text of two bindings: the columns in the header, then one cell per binding, bar between.
    let text = run_in(&t.db(), &["query", "FIND user AS u JOIN team AS t VIA member_of WHERE u.age = 41"]).out;
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("1 row(s) of (u, t); first binding read by FieldIndex(age)"), "{text}");
    assert_eq!(
        lines[1],
        format!("u {} user {{age=41, email=\"bob@x\"}}  |  t {} team {{name=\"core\"}}", l.bob, l.core),
        "{text}"
    );
    assert_eq!(
        rows_of(&q("FIND team AS t JOIN user AS u VIA member_of IN WHERE u.age < 40 ORDER BY u.age")),
        [vec![l.ops.to_string(), l.cy.to_string()], vec![l.core.to_string(), l.ada.to_string()]]
    );
    // A joined unit with no schema shows its payload; a join through another relation type.
    let json = q("FIND user AS u JOIN note AS n VIA wrote");
    assert_eq!(rows_of(&json), [vec![l.ada.to_string(), l.note.to_string()]]);
    assert_eq!(json["rows"][0][1]["payload"]["text"], "hello");
    assert_eq!(json["rows"][0][1]["revision"], 1);
    assert!(json["rows"][0][1].get("fields").is_none());
    let text = run_in(&t.db(), &["query", "FIND user AS u JOIN note AS n VIA wrote"]).out;
    assert!(text.contains(&format!("n {} note 5 bytes, text \"hello\"", l.note)), "{text}");
    // Joins from an earlier binding; a join may start from any earlier binding.
    let json = q("FIND user AS u JOIN team AS t VIA member_of JOIN note AS n VIA wrote FROM u");
    assert_eq!(rows_of(&json), [vec![l.ada.to_string(), l.core.to_string(), l.note.to_string()]]);
    assert_eq!(rows_of(&q("FIND user AS u JOIN team AS t VIA member_of LIMIT 2")).len(), 2);
}

#[test]
fn query_text_explains_the_plan_it_ran_and_the_planner_scan_under_it() {
    let t = TempDir::new();
    let l = build_lang(&t.db());
    let q = "FIND user AS u JOIN team AS t VIA member_of WHERE u.email = \"ada@x\"";
    let json = json_of(&t.db(), &["query", q, "--explain"]);
    let plan = &json["plan"];
    assert_eq!(plan["root"], serde_json::json!({ "alias": "u", "type": "user" }));
    assert_eq!(plan["chosen"], "FieldIndex(email)");
    assert_eq!(json["access"], "FieldIndex(email)");
    let candidates = plan["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!((candidates[0]["access"].as_str(), candidates[0]["chosen"].as_bool()), (Some("TypeScan"), Some(false)));
    assert_eq!(
        (candidates[1]["access"].as_str(), candidates[1]["chosen"].as_bool()),
        (Some("FieldIndex(email)"), Some(true))
    );
    assert!(candidates[1]["cost"].as_u64() < candidates[0]["cost"].as_u64(), "{candidates:?}");
    assert_eq!(
        plan["joins"],
        serde_json::json!([{ "alias": "t", "type": "team", "relation_type": "member_of", "direction": "out", "from": "u" }])
    );
    // The planner's own plan for a scan of the type is inside, with its fingerprint and candidates.
    assert!(plan["scan"]["fingerprint"].as_str().unwrap().starts_with("units/blake3-keyed/v1/"), "{plan}");
    assert_eq!(plan["scan"]["candidates"].as_array().unwrap().len(), 3);
    assert_eq!(plan["planner_version"], kerno_units::PLANNER_VERSION);
    assert_eq!(json["scan_path"], Value::Null, "a field index read is not a planner scan");
    let scanned = json_of(&t.db(), &["query", "FIND team AS t", "--explain"]);
    assert_eq!((scanned["access"].as_str(), scanned["scan_path"].as_str()), (Some("TypeScan"), Some("UnitsReference")));
    assert_eq!(rows_of(&json), [vec![l.ada.to_string(), l.core.to_string()]]);

    let text = run_in(&t.db(), &["query", q, "--explain"]).out;
    for want in [
        "plan: u (user), planner v",
        "candidate TypeScan cost ",
        "chosen    FieldIndex(email) cost ",
        "join t (team) via \"member_of\" out from u: a relation lookup per row, planned",
        "first binding scanned, if it is, through the planner's own plan:\nplan units/blake3-keyed/v1/",
        "candidate UnitsReference cost",
    ] {
        assert!(text.contains(want), "{want}: {text}");
    }
    // An in-direction join says so, and a condition the index cannot use leaves the scan the only candidate.
    let json = json_of(
        &t.db(),
        &["query", "FIND team AS t JOIN user AS u VIA member_of IN WHERE t.name != \"x\"", "--explain"],
    );
    assert_eq!(json["plan"]["joins"][0]["direction"], "in");
    assert_eq!(json["plan"]["chosen"], "TypeScan");
    assert_eq!(json["plan"]["candidates"].as_array().unwrap().len(), 1);
}

#[test]
fn query_text_that_is_wrong_is_a_usage_error_and_one_that_cannot_run_is_a_failure() {
    let t = TempDir::new();
    build_lang(&t.db());
    let db = t.db();
    let d = db.to_str().unwrap();
    let one_line = |r: &Run, code: i32| {
        assert_eq!(r.code, code, "{} {}", r.out, r.err);
        assert!(r.out.is_empty(), "{}", r.out);
        assert!(r.err.starts_with("kerno: ") && r.err.trim_end().lines().count() == 1, "{:?}", r.err);
    };
    // The query does not parse: the byte it stopped at.
    let r = kerno(&[d, "query", "FIND user WHERE"]);
    one_line(&r, 2);
    assert_eq!(r.err.trim_end(), "kerno: query: at byte 15: expected a field, found the end of the query");
    one_line(&kerno(&[d, "query", "SELECT * FROM user"]), 2);
    // Not both a query and --type; --limit is for --type; two queries; neither.
    for args in [
        vec![d, "query", "FIND user", "--type", "user"],
        vec![d, "query", "FIND user", "--limit", "1"],
        vec![d, "query", "FIND user", "FIND team"],
        vec![d, "query", "--limit", "1"],
    ] {
        one_line(&kerno(&args), 2);
    }
    // It parses and does not fit the schemas, or the database is not there: a failure.
    let r = kerno(&[d, "query", "FIND user WHERE nope = 1"]);
    one_line(&r, 1);
    assert!(r.err.contains("declares no such field"), "{}", r.err);
    one_line(&kerno(&[d, "query", "FIND user WHERE age = \"x\""]), 1);
    one_line(&kerno(&[d, "query", "FIND note WHERE x = 1"]), 1);
    let missing = t.path("missing");
    one_line(&run_in(&missing, &["query", "FIND user"]), 1);
    assert!(!missing.exists());
    // The --type form is untouched.
    assert_eq!(json_of(&t.db(), &["query", "--type", "user"])["total"], 3);
    // The help says what the query takes.
    let help = run_in(&t.db(), &["query", "--help"]).out;
    assert!(help.contains("docs/query-language.md") && help.contains("--type T"), "{help}");
}

// ------------------------------------------- export, import, reclaim

#[test]
fn export_then_import_makes_the_same_database_and_the_same_export() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    let (first, copy, second) = (t.path("first.kexp"), t.path("copy"), t.path("second.kexp"));
    let r = run_in(&t.db(), &["export", first.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("(export format v1)") && !r.out.contains("draft"), "{}", r.out);
    assert!(std::fs::read(&first).unwrap().starts_with(b"KUNITEXD"));

    // The two forms of import: without a source directory, and with one that is ignored.
    let r = kerno(&["import", first.to_str().unwrap(), copy.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains("imported 5 unit(s)") && r.out.contains("2 relation(s)") && r.out.contains("1 public id(s)"),
        "{}",
        r.out
    );
    assert_eq!(run_in(&copy, &["verify", "--deep"]).code, 0);
    assert_eq!(run_in(&copy, &["export", second.to_str().unwrap()]).code, 0);
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
        "I-30: the same logical state, the same bytes"
    );
    // The copy holds the data, the history and the public id.
    assert_eq!(json_of(&copy, &["get", &fx.public_a])["payload"]["text"], "alpha two");
    assert_eq!(json_of(&copy, &["history", &fx.note_a.to_string()])["revisions"].as_array().unwrap().len(), 2);
    assert_eq!(json_of(&copy, &["stats"])["relations"]["total"], 2);

    let other = t.path("other");
    let r = run_in(&t.path("ignored"), &["import", first.to_str().unwrap(), other.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(!t.path("ignored").exists());
    assert_eq!(files_of(&other).keys().collect::<Vec<_>>(), files_of(&copy).keys().collect::<Vec<_>>());
}

#[test]
fn import_refuses_a_directory_in_use_and_a_bad_file_and_leaves_nothing_behind() {
    let t = TempDir::new();
    build(&t.db(), true);
    let file = t.path("good.kexp");
    assert_eq!(run_in(&t.db(), &["export", file.to_str().unwrap()]).code, 0);
    // Into a directory that holds something: refused, untouched.
    let before = files_of(&t.db());
    let r = kerno(&["import", file.to_str().unwrap(), t.db().to_str().unwrap()]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert_eq!(files_of(&t.db()), before);
    // A file that is not an export: refused, and the directory this call would have made is gone.
    let junk = t.path("junk.kexp");
    std::fs::write(&junk, b"this is not an export").unwrap();
    let target = t.path("target");
    let r = kerno(&["import", junk.to_str().unwrap(), target.to_str().unwrap()]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.err.starts_with("kerno: "), "{}", r.err);
    assert!(!target.exists());
    // An empty directory that existed stays, empty.
    std::fs::create_dir(&target).unwrap();
    assert_eq!(kerno(&["import", junk.to_str().unwrap(), target.to_str().unwrap()]).code, 1);
    assert!(target.is_dir() && files_of(&target).is_empty());
    // The file to import must exist.
    assert_eq!(kerno(&["import", t.path("nope").to_str().unwrap(), t.path("fresh").to_str().unwrap()]).code, 1);
    assert!(!t.path("fresh").exists());
}

#[test]
fn reclaim_compacts_the_live_files_keeps_the_data_and_says_it_is_not_an_erase() {
    let t = TempDir::new();
    build_with_tables(&t.db(), false);
    let levels = |t: &TempDir| -> Vec<u64> {
        let json = json_of(&t.db(), &["verify"]);
        json["engine"]["levels"].as_array().unwrap().iter().map(|l| l["files"].as_u64().unwrap()).collect()
    };
    assert!(levels(&t)[..6].iter().sum::<u64>() > 0, "the fixture starts with tables above the last level");
    let before = json_of(&t.db(), &["stats"]);
    assert_eq!(before["units"]["by_type"]["blob"]["active"], 300);
    let r = run_in(&t.db(), &["reclaim"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("not a secure erase"), "{}", r.out);
    // Everything the live files hold is now in one table of the last level.
    assert_eq!(levels(&t), [0, 0, 0, 0, 0, 0, 1]);
    let after = json_of(&t.db(), &["stats"]);
    assert_eq!(after["units"], before["units"]);
    assert_eq!(run_in(&t.db(), &["verify", "--deep"]).code, 0);
    assert_eq!(run_in(&t.db(), &["reclaim", "extra"]).code, 2);
}

// ------------------------------------------------------------ read-only

#[test]
fn the_read_commands_leave_every_file_byte_identical() {
    let t = TempDir::new();
    let fx = build(&t.db(), true);
    let before = files_of(&t.db());
    let (a, export) = (fx.note_a.to_string(), t.path("out.kexp"));
    let commands: Vec<Vec<&str>> = vec![
        vec!["verify"],
        vec!["verify", "--deep"],
        vec!["stats"],
        vec!["get", &a],
        vec!["history", &a],
        vec!["relations", &a],
        vec!["query", "--type", "note", "--explain"],
        vec!["export", export.to_str().unwrap()],
    ];
    for args in commands {
        let r = run_in(&t.db(), &args);
        assert_eq!(r.code, 0, "{args:?}: {} {}", r.out, r.err);
        assert_eq!(files_of(&t.db()), before, "{args:?} changed the directory");
    }
    // The same holds when the index was never built: the planner falls back, it does not build.
    let unbuilt = TempDir::new();
    build(&unbuilt.db(), false);
    let before = files_of(&unbuilt.db());
    assert_eq!(run_in(&unbuilt.db(), &["query", "--type", "note"]).code, 0);
    assert_eq!(files_of(&unbuilt.db()), before);
}

#[test]
fn a_read_command_cannot_open_a_database_another_handle_holds() {
    let t = TempDir::new();
    build(&t.db(), true);
    let open = open_units(&t.db(), None);
    for args in [&["stats"][..], &["verify"], &["query", "--type", "note"]] {
        let r = kerno_once(&[&[t.db().to_str().unwrap()], args].concat());
        assert_eq!(r.code, 1, "{args:?}: {} {}", r.out, r.err);
        assert!(r.err.starts_with("kerno: ") && r.err.contains("locked by another handle"), "{}", r.err);
    }
    close_and_wait(open, &t.db());
    assert_eq!(run_in(&t.db(), &["stats"]).code, 0);
}

// ------------------------------------------------ backup, ship, restore

/// Like `open_units`, with ids that do not collide with the ones `build` made.
fn open_more(dir: &Path) -> UnitDb<KernoDb> {
    let engine = open_engine(dir, &Config::default());
    let clock = Arc::new(FixedClock::stepping(Timestamp(9_000_000), 1000));
    UnitDb::from_engine(engine, &UnitConfig::with_sources(clock, Arc::new(CounterIds::new(1000)))).unwrap()
}

fn url_of(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// One more change set on an open database.
fn write_more(db: &UnitDb<KernoDb>, payloads: &[&str]) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for p in payloads {
        cs.create(new_unit("note", p.as_bytes())).unwrap();
    }
    cs.commit().unwrap();
}

/// The export file of a database, to compare two of them byte for byte.
fn export_of(dir: &Path, name: &str, t: &TempDir) -> Vec<u8> {
    let file = t.path(name);
    let r = run_in(dir, &["export", file.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}", r.err);
    std::fs::read(&file).unwrap()
}

#[test]
fn backup_ship_and_restore_rebuild_the_database_while_a_handle_holds_the_original() {
    let t = TempDir::new();
    build(&t.db(), true);
    let url = url_of(&t.path("store"));
    let at_base = export_of(&t.db(), "at-base.kexp", &t);

    let text = run_in(&t.db(), &["backup", &url]).out;
    assert!(text.starts_with("backed up commit ") && text.contains(&format!("to {url}")), "{text}");
    // The same commit again replaces the base (its time is the later one).
    let base = json_of(&t.db(), &["backup", &url]);
    assert!(base["seq"].as_u64().unwrap() > 0 && base["entries"].as_u64().unwrap() > 0, "{base}");
    assert_eq!(base["parts"], 1);
    std::thread::sleep(std::time::Duration::from_millis(20));

    // A handle that holds the directory blocks `backup`, which opens it, but not `ship`, which only reads files.
    let held = open_more(&t.db());
    write_more(&held, &["first", "second"]);
    write_more(&held, &["third"]);
    let r = kerno_once(&[t.db().to_str().unwrap(), "backup", &url]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.err.contains("locked by another handle"), "{}", r.err);
    let shipped = json_of(&t.db(), &["ship", &url]);
    assert!(shipped["commits"].as_u64().unwrap() > 0 && shipped["segments"] == 1, "{shipped}");
    assert_eq!(shipped["shipped_seq"].as_u64().unwrap(), held.raw_seq().unwrap());
    let r = run_in(&t.db(), &["ship", &url]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.starts_with("shipped 0 commit(s) in 0 segment(s)") && r.out.contains("complete up to commit"),
        "{}",
        r.out
    );
    close_and_wait(held, &t.db());
    let latest = export_of(&t.db(), "latest.kexp", &t);
    assert_ne!(latest, at_base);

    // The whole thing.
    let dest = t.path("dest");
    let r = run_in(&dest, &["restore", &url]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.starts_with(&format!("restored {} at commit ", dest.display()))
            && r.out.contains("from the base at commit")
            && r.out.contains("replayed commit(s)"),
        "{}",
        r.out
    );
    assert_eq!(run_in(&dest, &["verify", "--deep"]).code, 0);
    assert_eq!(export_of(&dest, "dest.kexp", &t), latest);

    // To the commit of the base, and to the moment of the base.
    let seq = base["seq"].as_u64().unwrap().to_string();
    let to_seq = t.path("to-seq");
    let r = json_of(&to_seq, &["restore", &url, "--to-seq", &seq]);
    assert_eq!((r["seq"].as_u64().unwrap().to_string(), r["replayed"].as_u64().unwrap()), (seq.clone(), 0));
    assert_eq!(export_of(&to_seq, "to-seq.kexp", &t), at_base);
    let created = base["created_ms"].as_u64().unwrap();
    let to_time = t.path("to-time");
    assert_eq!(run_in(&to_time, &["restore", &url, "--to-time", &created.to_string()]).code, 0);
    assert_eq!(export_of(&to_time, "to-time.kexp", &t), at_base);
    // A time before any base has nothing to start from.
    let early = run_in(&t.path("early"), &["restore", &url, "--to-time", &(created - 1).to_string()]);
    assert_eq!(early.code, 1, "{}", early.out);
    assert!(early.err.contains("no base backup taken at or before"), "{}", early.err);
    assert!(!t.path("early").exists());
}

#[test]
fn ship_needs_a_base_and_says_when_a_new_one_is_needed() {
    let t = TempDir::new();
    build(&t.db(), true);
    let url = url_of(&t.path("store"));
    let r = run_in(&t.db(), &["ship", &url]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert_eq!(r.err.trim_end(), "kerno: the store has no base backup: take one first");

    assert_eq!(run_in(&t.db(), &["backup", &url]).code, 0);
    // The clean close that follows deletes the log: commits made before it were never shipped.
    let db = open_more(&t.db());
    write_more(&db, &["lost"]);
    close_and_wait(db, &t.db());
    let held = open_more(&t.db());
    let r = run_in(&t.db(), &["ship", &url]);
    close_and_wait(held, &t.db());
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.err.contains("no longer holds commit") && r.err.contains("take a new base backup"), "{}", r.err);
}

#[test]
fn restore_refuses_a_destination_in_use_and_a_point_the_backup_does_not_reach() {
    let t = TempDir::new();
    build(&t.db(), true);
    let url = url_of(&t.path("store"));
    let base = json_of(&t.db(), &["backup", &url]);

    let before = files_of(&t.db());
    let r = run_in(&t.db(), &["restore", &url]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.err.contains("is not empty"), "{}", r.err);
    assert_eq!(files_of(&t.db()), before);

    let past = (base["seq"].as_u64().unwrap() + 5).to_string();
    let r = run_in(&t.path("far"), &["restore", &url, "--to-seq", &past]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert_eq!(r.err.trim_end(), format!("kerno: the backup reaches commit {} only, not {past}", base["seq"]));
    assert!(!t.path("far").exists() && !t.path("far.restoring").exists());

    let r = run_in(&t.path("empty-store"), &["restore", &url_of(&t.path("nothing"))]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert_eq!(r.err.trim_end(), "kerno: the store has no base backup");
}

#[test]
fn backups_lists_what_a_store_holds_without_a_database_directory() {
    let t = TempDir::new();
    build(&t.db(), true);
    let url = url_of(&t.path("store"));
    let ignored = t.path("not-a-database");

    let r = run_in(&ignored, &["backups", &url]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, format!("{url}\nno base backup\n0 segment(s) shipped\n"));
    assert!(!ignored.exists());

    let base = json_of(&t.db(), &["backup", &url]);
    let held = open_more(&t.db());
    write_more(&held, &["one"]);
    assert_eq!(run_in(&t.db(), &["ship", &url]).code, 0);
    let end = held.raw_seq().unwrap();
    close_and_wait(held, &t.db());

    let r = run_in(&ignored, &["backups", &url]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains(&format!("base at commit {}: taken at {} ms", base["seq"], base["created_ms"]))
            && r.out.contains(&format!("a restore reaches commit {end}"))
            && r.out.ends_with("1 segment(s) shipped\n"),
        "{}",
        r.out
    );
    let json = json_of(&ignored, &["backups", &url]);
    assert_eq!((json["bases"][0]["seq"].clone(), json["reach"][0].as_u64()), (base["seq"].clone(), Some(end)));
    assert_eq!(json["segments"].as_array().unwrap().len(), 1);
    assert_eq!(json["segments"][0]["last"], end);
}

#[test]
fn the_backup_commands_say_what_is_wrong_with_the_line_or_the_location() {
    let t = TempDir::new();
    build(&t.db(), true);
    let d = t.db();
    let d = d.to_str().unwrap();
    let url = url_of(&t.path("store"));
    let usage = |args: &[&str]| {
        let r = kerno(args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        r.err.trim_end().to_owned()
    };
    assert_eq!(usage(&[d, "backup"]), "kerno: backup: expected 1 argument(s), got 0 (usage: backup <url> [--json])");
    assert_eq!(
        usage(&[d, "ship", "a", "b"]),
        "kerno: ship: expected 1 argument(s), got 2 (usage: ship <url> [--json])"
    );
    assert_eq!(
        usage(&[d, "restore"]),
        "kerno: restore: expected 1 argument(s), got 0 (usage: restore <url> [--to-seq N | --to-time MS] [--json])"
    );
    assert_eq!(usage(&[d, "backups"]), "kerno: backups: expected 1 argument(s), got 0 (usage: backups <url> [--json])");
    assert_eq!(usage(&[d, "restore", &url, "--to-seq"]), "kerno: restore: --to-seq needs a value");
    assert_eq!(
        usage(&[d, "restore", &url, "--to-seq", "x1"]),
        "kerno: restore: --to-seq \"x1\" is not a commit number"
    );
    assert_eq!(
        usage(&[d, "restore", &url, "--to-time", "-5"]),
        "kerno: restore: --to-time \"-5\" is not a time in milliseconds since the Unix epoch"
    );
    assert_eq!(
        usage(&[d, "restore", &url, "--to-seq", "1", "--to-time", "2"]),
        "kerno: restore: give --to-seq or --to-time, not both"
    );

    // A location that is not one is a failure, and a failed backup leaves the database as it was.
    let before = files_of(&t.db());
    for bad in ["ftp://host/x", "relative/dir", "file://relative"] {
        for command in ["backup", "ship", "backups"] {
            let r = run_in(&t.db(), &[command, bad]);
            assert_eq!(r.code, 1, "{command} {bad}: {}", r.out);
            assert!(r.err.starts_with("kerno: cannot open the backup location: "), "{command} {bad}: {}", r.err);
        }
    }
    assert_eq!(files_of(&t.db()), before);
}
