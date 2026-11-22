//! End to end: every test starts the real `kerno-mcp` binary and talks to it
//! over its stdin and stdout, one JSON message per line.

// The no-panic rule is for the server; a test panics to fail.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Value, json};

/// A fresh directory that is removed when the test ends, pass or fail.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kerno-mcp-e2e-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    /// Two levels that do not exist yet: the server must create both.
    fn db(&self) -> PathBuf {
        self.0.join("memory").join("db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn binary() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kerno-mcp"));
    cmd.env_remove("KERNO_MCP_DB").env_remove(kerno_storage::REQUIRE_VERIFIED_ENV);
    cmd
}

struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Session {
    fn start(mut cmd: Command) -> Self {
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session { child, stdin, stdout, next_id: 1 }
    }

    fn open(db: &TempDir) -> Self {
        let mut cmd = binary();
        cmd.arg("--db").arg(db.db());
        Self::start(cmd)
    }

    fn send_raw(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        assert!(self.stdout.read_line(&mut line).unwrap() > 0, "the server closed its output");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line:?}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string());
        let answer = self.recv();
        assert_eq!(answer["id"], json!(id), "answer to another request: {answer}");
        answer
    }

    /// The full answer to `tools/call`.
    fn call_raw(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({ "name": tool, "arguments": args }))
    }

    /// A call that must succeed: the JSON the tool returned.
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let answer = self.call_raw(tool, args);
        assert_eq!(answer["result"]["isError"], json!(false), "{tool} failed: {answer}");
        serde_json::from_str(answer["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    /// A call that must fail as a tool result: its message.
    fn call_fails(&mut self, tool: &str, args: Value) -> String {
        let answer = self.call_raw(tool, args);
        assert_eq!(answer["result"]["isError"], json!(true), "{tool} should have failed: {answer}");
        answer["result"]["content"][0]["text"].as_str().unwrap().to_owned()
    }

    /// Closes stdin and waits for the server to end; its exit status.
    fn finish(mut self) -> bool {
        drop(self.stdin.take());
        self.child.wait().unwrap().success()
    }
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v[key].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn handshake_lists_the_tools_and_answers_ping() {
    let dir = TempDir::new();
    let mut s = Session::open(&dir);

    let init = s.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" }
        }),
    );
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert_eq!(init["result"]["serverInfo"]["name"], "kerno-mcp");

    // A notification gets no answer: the next line is the answer to the ping.
    s.send_raw(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    // Not even a wrong one: it has no id to answer to.
    s.send_raw(r#"{"method":"notifications/initialized"}"#);
    // Nor a blank line, nor a response (this server sends no requests to answer).
    s.send_raw("");
    s.send_raw(r#"{"jsonrpc":"2.0","id":99,"result":{}}"#);
    assert_eq!(s.request("ping", json!({}))["result"], json!({}));
    // A request without `jsonrpc: "2.0"` is refused, under its own id.
    s.send_raw(r#"{"id":"x","method":"ping"}"#);
    let e = s.recv();
    assert_eq!((e["id"].as_str(), e["error"]["code"].as_i64()), (Some("x"), Some(-32600)), "{e}");

    let list = s.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let mut found: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    found.sort_unstable();
    assert_eq!(found, ["forget", "get", "history", "recall", "relate", "remember", "verify"]);
    for t in tools {
        assert!(!t["description"].as_str().unwrap().is_empty());
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
    }
    let remember = tools.iter().find(|t| t["name"] == "remember").unwrap();
    assert_eq!(remember["inputSchema"]["required"], json!(["type", "text"]));
    assert!(s.finish());
}

#[test]
fn forget_says_plainly_that_deletion_is_logical() {
    let dir = TempDir::new();
    let mut s = Session::open(&dir);
    let list = s.request("tools/list", json!({}));
    let forget = list["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "forget").unwrap().clone();
    let description = forget["description"].as_str().unwrap();
    assert!(description.contains("LOGICAL"), "{description}");
    assert!(description.contains("stay on disk until the engine compacts"), "{description}");
    assert!(description.contains("ADR-0009"), "{description}");
    assert!(s.finish());
}

#[test]
fn remember_relate_recall_forget_round_trip() {
    let dir = TempDir::new();
    let mut s = Session::open(&dir);

    let a = s.call("remember", json!({ "type": "decision", "text": "use LSM storage", "tags": ["storage", "adr"] }));
    let b = s.call("remember", json!({ "type": "fact", "text": "LSM compacts in the background" }));
    let (a_id, b_id) = (a["id"].as_str().unwrap().to_owned(), b["id"].as_str().unwrap().to_owned());
    assert!(a_id.starts_with("k1_") && a_id != b_id, "{a_id} {b_id}");

    let rel = s.call("relate", json!({ "from": a_id, "to": b_id, "relation_type": "supported_by" }));
    assert_eq!(rel["relation_type"], "supported_by");

    // Associative query: B is one hop from A, with the relation that led to it.
    let out = s.call("recall", json!({ "from": a_id }));
    assert_eq!(names(&out, "hits"), vec![b_id.clone()]);
    let hit = &out["hits"][0];
    assert_eq!(
        (hit["text"].as_str().unwrap(), hit["distance"].as_u64().unwrap()),
        ("LSM compacts in the background", 1)
    );
    assert_eq!(hit["tags"], json!([]), "no tags reads as none, not as one empty tag: {hit}");
    assert_eq!(hit["path"][0]["relation_type"], "supported_by");
    assert_eq!(hit["path"][0]["relation_id"], rel["relation_id"]);
    assert_eq!(
        (hit["path"][0]["from"].as_str().unwrap(), hit["path"][0]["to"].as_str().unwrap()),
        (a_id.as_str(), b_id.as_str())
    );
    assert_eq!(out["ending"], "Exhausted");
    // By default the walk follows relations both ways; direction and relation type narrow it.
    assert_eq!(names(&s.call("recall", json!({ "from": b_id })), "hits"), vec![a_id.clone()]);
    assert_eq!(names(&s.call("recall", json!({ "from": b_id, "direction": "in" })), "hits"), vec![a_id.clone()]);
    assert!(names(&s.call("recall", json!({ "from": b_id, "direction": "out" })), "hits").is_empty());
    assert!(names(&s.call("recall", json!({ "from": a_id, "relation_types": ["other"] })), "hits").is_empty());

    // By type and by tag.
    assert_eq!(names(&s.call("recall", json!({ "type": "fact" })), "units"), vec![b_id.clone()]);
    assert_eq!(names(&s.call("recall", json!({ "tag": "adr" })), "units"), vec![a_id.clone()]);
    assert!(names(&s.call("recall", json!({ "type": "fact", "tag": "adr" })), "units").is_empty());

    let got = s.call("get", json!({ "id": a_id }));
    assert_eq!(got["text"], "use LSM storage");
    assert_eq!((got["type"].as_str().unwrap(), got["revision"].as_u64().unwrap()), ("decision", 1));
    assert_eq!(got["tags"], json!(["storage", "adr"]));
    assert!(got["created_at_us"].as_u64().unwrap() > 0, "{got}");
    assert_eq!(got["created_at_us"], got["updated_at_us"], "{got}");
    let history = s.call("history", json!({ "id": a_id }));
    assert_eq!(history["revisions"].as_array().unwrap().len(), 1);
    let first = &history["revisions"][0];
    assert_eq!((first["revision"].as_u64(), first["kind"].as_str()), (Some(1), Some("Create")), "{history}");
    assert_eq!((&first["text"], &first["tags"]), (&json!("use LSM storage"), &json!(["storage", "adr"])));

    // A -> B -> C -> D: the default depth (2) reaches C and stops before D.
    let c = s.call("remember", json!({ "type": "fact", "text": "compaction rewrites files", "tags": ["adr"] }));
    let c_id = c["id"].as_str().unwrap().to_owned();
    let d = s.call("remember", json!({ "type": "note", "text": "files are immutable once written" }));
    let d_id = d["id"].as_str().unwrap().to_owned();
    s.call("relate", json!({ "from": b_id, "to": c_id, "relation_type": "explained_by" }));
    s.call("relate", json!({ "from": c_id, "to": d_id, "relation_type": "explained_by" }));
    let out = s.call("recall", json!({ "from": a_id }));
    assert_eq!(names(&out, "hits"), vec![b_id.clone(), c_id.clone()]);
    assert_eq!(out["hits"][1]["distance"], 2);
    assert!(out["relations_examined"].as_u64().unwrap() >= 2, "{out}");
    assert_eq!(names(&s.call("recall", json!({ "from": a_id, "max_depth": 1 })), "hits"), vec![b_id.clone()]);
    let deep = s.call("recall", json!({ "from": a_id, "max_depth": 3 }));
    assert_eq!(names(&deep, "hits"), vec![b_id.clone(), c_id.clone(), d_id.clone()]);

    // A listing is newest first and says when its limit cut it.
    let facts = s.call("recall", json!({ "type": "fact" }));
    assert_eq!((names(&facts, "units"), &facts["truncated"]), (vec![c_id.clone(), b_id.clone()], &json!(false)));
    let newest = s.call("recall", json!({ "type": "fact", "limit": 1 }));
    assert_eq!((names(&newest, "units"), &newest["truncated"]), (vec![c_id.clone()], &json!(true)));

    // Forgetting B removes the relation into it (from A), the one out of it (to C)
    // and its relation to itself, which counts once.
    s.call("relate", json!({ "from": b_id, "to": b_id, "relation_type": "self" }));
    let forgotten = s.call("forget", json!({ "id": b_id }));
    assert_eq!(forgotten["relations_removed"], 3, "{forgotten}");
    assert!(s.call_fails("get", json!({ "id": b_id })).contains("never stored, or it was forgotten"));
    assert!(s.call_fails("forget", json!({ "id": b_id })).contains(&b_id));
    assert!(s.call_fails("recall", json!({ "from": b_id })).contains(&b_id));
    assert!(names(&s.call("recall", json!({ "from": a_id })), "hits").is_empty());
    assert!(names(&s.call("recall", json!({ "from": c_id, "direction": "in" })), "hits").is_empty());
    assert_eq!(names(&s.call("recall", json!({ "type": "fact" })), "units"), vec![c_id.clone()]);
    assert_eq!(s.call("get", json!({ "id": a_id }))["text"], "use LSM storage");
    assert_eq!(s.call("get", json!({ "id": c_id }))["text"], "compaction rewrites files");

    // Forgetting A, which only starts relations, works too once it has one again.
    s.call("relate", json!({ "from": a_id, "to": c_id, "relation_type": "supported_by" }));
    assert_eq!(s.call("forget", json!({ "id": a_id }))["relations_removed"], 1);
    assert!(names(&s.call("recall", json!({ "from": c_id, "direction": "in" })), "hits").is_empty());

    let report = s.call("verify", json!({}));
    assert_eq!(report["clean"], true, "{report}");
    assert_eq!(report["counts"]["units"], 4, "a forgotten unit keeps its tombstone: {report}");
    assert!(s.finish());
}

#[test]
fn malformed_json_and_unknown_names_are_errors_not_crashes() {
    let dir = TempDir::new();
    let mut s = Session::open(&dir);

    s.send_raw("{not json");
    let e = s.recv();
    assert_eq!((e["error"]["code"].as_i64(), &e["id"]), (Some(-32700), &Value::Null), "{e}");
    assert!(e["error"]["message"].as_str().unwrap().starts_with("parse error"), "{e}");

    s.send_raw("[1, 2]");
    assert_eq!(s.recv()["error"]["code"], -32600);

    let e = s.request("no/such/method", json!({}));
    assert_eq!(e["error"]["code"], -32601);
    assert!(e["error"]["message"].as_str().unwrap().contains("no/such/method"), "{e}");

    let e = s.call_raw("no_such_tool", json!({}));
    assert_eq!(e["error"]["code"], -32602, "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("no_such_tool"), "{e}");
    assert!(e["result"].is_null());

    // Bad arguments: a missing field, a field nobody declared, an id that is not one.
    let e = s.call_raw("remember", json!({ "type": "fact" }));
    assert_eq!(e["error"]["code"], -32602);
    assert!(e["error"]["message"].as_str().unwrap().contains("text"), "{e}");
    assert_eq!(s.call_raw("get", json!({ "id": "k1_x", "extra": 1 }))["error"]["code"], -32602);
    let e = s.call_raw("get", json!({ "id": "not-an-id" }));
    assert_eq!(e["error"]["code"], -32602);
    // A well formed id that names nothing is a failed call the model can read.
    s.call_fails("get", json!({ "id": "k1_aaaaaaaaaaaaaaaaaaaaaaaaaa" }));
    // Tags are newline-joined on disk, so a newline or an empty tag would change them;
    // all of them must fit the engine's 4096 bytes of metadata.
    for (tags, word) in [(json!(["a\nb"]), "newline"), (json!([""]), "empty"), (json!(["t".repeat(4097)]), "4096")] {
        let e = s.call_raw("remember", json!({ "type": "fact", "text": "x", "tags": tags }));
        assert_eq!(e["error"]["code"], -32602, "{e}");
        assert!(e["error"]["message"].as_str().unwrap().contains(word), "{e}");
    }

    // A request id must be a string or a number; a request needs a method.
    s.send_raw(r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#);
    let e = s.recv();
    assert_eq!((&e["id"], e["error"]["code"].as_i64()), (&Value::Null, Some(-32600)), "{e}");
    s.send_raw(r#"{"jsonrpc":"2.0","id":7}"#);
    let e = s.recv();
    assert_eq!((e["id"].as_i64(), e["error"]["code"].as_i64()), (Some(7), Some(-32600)), "{e}");
    // `tools/call` without `arguments` is a call with none.
    let e = s.request("tools/call", json!({ "name": "verify" }));
    assert_eq!(e["result"]["isError"], false, "{e}");

    // The server is still there.
    assert_eq!(s.request("ping", json!({}))["result"], json!({}));
    assert!(s.finish());
}

#[test]
fn recall_refuses_what_is_past_the_engines_hard_bounds() {
    let dir = TempDir::new();
    let mut s = Session::open(&dir);
    let id = s.call("remember", json!({ "type": "fact", "text": "x" }))["id"].as_str().unwrap().to_owned();

    for (args, word) in [
        (json!({ "from": id, "max_depth": 5 }), "max_depth"),
        (json!({ "from": id, "max_depth": 0 }), "max_depth"),
        (json!({ "from": id, "limit": 1001 }), "limit"),
        (json!({ "from": id, "visit_budget": 100_001 }), "visit_budget"),
        (json!({ "type": "fact", "limit": 1001 }), "limit"),
        (json!({}), "from"),
        (json!({ "from": id, "tag": "t" }), "not both"),
        (json!({ "from": id, "type": "fact" }), "not both"),
        (json!({ "tag": "t", "max_depth": 2 }), "from"),
        (json!({ "tag": "t", "direction": "in" }), "from"),
        (json!({ "type": "fact", "relation_types": ["r"] }), "from"),
        (json!({ "type": "fact", "visit_budget": 5 }), "from"),
    ] {
        let e = s.call_raw("recall", args.clone());
        assert_eq!(e["error"]["code"], -32602, "{args}: {e}");
        assert!(e["error"]["message"].as_str().unwrap().contains(word), "{args}: {e}");
    }
    // The maxima themselves are accepted.
    s.call("recall", json!({ "from": id, "max_depth": 4, "limit": 1000, "visit_budget": 100_000 }));
    // `history` takes the same limit.
    for limit in [0, 1001] {
        let e = s.call_raw("history", json!({ "id": id, "limit": limit }));
        assert_eq!(e["error"]["code"], -32602, "{e}");
    }
    assert_eq!(s.call("history", json!({ "id": id, "limit": 1000 }))["revisions"].as_array().unwrap().len(), 1);
    assert!(s.finish());
}

#[test]
fn memories_survive_a_restart_and_the_directory_may_come_from_the_environment() {
    let dir = TempDir::new();
    let mut cmd = binary();
    cmd.arg(format!("--db={}", dir.db().display()));
    let mut first = Session::start(cmd);
    let id = first.call("remember", json!({ "type": "preference", "text": "answer in Spanish" }))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(first.finish());

    let mut cmd = binary();
    cmd.env("KERNO_MCP_DB", dir.db());
    let mut second = Session::start(cmd);
    let got = second.call("get", json!({ "id": id }));
    assert_eq!(got["text"], "answer in Spanish");
    assert_eq!(got["id"], json!(id));
    assert!(second.finish());

    // The flag wins over the environment.
    let other = TempDir::new();
    let mut cmd = binary();
    cmd.arg("--db").arg(dir.db()).env("KERNO_MCP_DB", other.db());
    let mut third = Session::start(cmd);
    assert_eq!(third.call("get", json!({ "id": id }))["text"], "answer in Spanish");
    assert!(third.finish());
}

#[test]
fn without_a_database_the_server_says_so_and_exits() {
    for (args, env, word) in [
        (&[][..], None, "no database"),
        (&[][..], Some(""), "no database"),
        (&["--bogus"][..], None, "unknown argument"),
        (&["--db"][..], None, "--db needs a directory"),
    ] {
        let mut cmd = binary();
        cmd.args(args);
        if let Some(v) = env {
            cmd.env("KERNO_MCP_DB", v);
        }
        let out = cmd.stdin(Stdio::null()).output().unwrap();
        assert!(!out.status.success(), "{args:?} {env:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(word), "{args:?} {env:?}: {stderr}");
        assert!(out.stdout.is_empty(), "nothing but protocol may reach stdout");
    }

    // Help is asked for, so it goes to stdout and the exit is a success.
    let out = binary().arg("--help").stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("usage: kerno-mcp --db <dir>"));
}

/// Windows, macOS and the rest do not verify `Sync` durability: the server must still start there
/// (issue #7), say so on stderr, and refuse only when asked to.
#[test]
fn unverified_durability_is_a_warning_and_the_environment_can_refuse_to_run() {
    let verified = cfg!(target_os = "linux");
    let warning = format!("kerno-mcp: warning: {}", kerno_storage::UNVERIFIED_WARNING);
    // stdin at its end: the server opens the database, finds no request and exits.
    let run = |require: Option<&str>| {
        let dir = TempDir::new();
        let mut cmd = binary();
        cmd.arg("--db").arg(dir.db());
        if let Some(v) = require {
            cmd.env(kerno_storage::REQUIRE_VERIFIED_ENV, v);
        }
        let out = cmd.stdin(Stdio::null()).output().unwrap();
        assert!(out.stdout.is_empty(), "nothing but protocol may reach stdout");
        (out.status.success(), String::from_utf8(out.stderr).unwrap())
    };
    let (ok, err) = run(None);
    assert!(ok, "{err}");
    assert_eq!(err.trim_end(), if verified { "" } else { warning.as_str() });
    for off in ["0", ""] {
        let (ok, err) = run(Some(off));
        assert!(ok, "{off:?}: {err}");
    }
    let (ok, err) = run(Some("1"));
    if verified {
        assert!((ok, err.as_str()) == (true, ""), "{err}");
    } else {
        assert!(!ok && err.contains("durability not verified on this platform") && !err.contains(&warning), "{err}");
    }
}
