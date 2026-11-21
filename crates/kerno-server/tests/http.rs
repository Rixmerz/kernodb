//! End to end: every test starts the real `kerno-server` binary and calls its HTTP API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Server, TempDir, raw_request};
use serde_json::{Value, json};

fn start() -> (TempDir, Server) {
    let dir = TempDir::new();
    let server = Server::start(&dir.db());
    (dir, server)
}

/// The id of a unit created with `text`.
fn note(server: &Server, text: &str) -> String {
    let reply = server.post("/v1/units", &json!({ "type": "note", "payload": { "text": text } }));
    assert_eq!(reply.status, 201, "{}", String::from_utf8_lossy(&reply.body));
    reply.json()["id"].as_str().unwrap().to_owned()
}

fn error_code(reply: &common::Reply) -> String {
    reply.json()["error"]["code"].as_str().unwrap().to_owned()
}

#[test]
fn health_answers_ok_with_the_version() {
    let (_dir, server) = start();
    let reply = server.get("/v1/health");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    assert_eq!(reply.json(), json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }));
}

#[test]
fn a_unit_is_created_and_read_back() {
    let (_dir, server) = start();
    let created = server.post("/v1/units", &json!({ "type": "note", "payload": { "text": "Rust has no GC" } }));
    assert_eq!(created.status, 201);
    let unit = created.json();
    let id = unit["id"].as_str().unwrap();
    assert_eq!(id.len(), 32);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert_eq!(unit["type"], "note");
    assert_eq!(unit["lifecycle"], "active");
    assert_eq!(unit["revision"], 1);
    assert_eq!(unit["payload"], json!({ "text": "Rust has no GC" }));
    assert!(unit["created_at_us"].as_i64().unwrap() > 0);

    let read = server.get(&format!("/v1/units/{id}"));
    assert_eq!(read.status, 200);
    assert_eq!(read.json(), unit);
}

#[test]
fn bytes_that_are_not_text_travel_as_hex() {
    let (_dir, server) = start();
    let created = server.post("/v1/units", &json!({ "type": "blob", "payload": { "hex": "00ff10" } }));
    assert_eq!(created.status, 201);
    assert_eq!(created.json()["payload"], json!({ "hex": "00ff10" }));
    // Bytes that happen to be UTF-8 read back as text.
    let text = server.post("/v1/units", &json!({ "type": "blob", "payload": { "hex": "6869" } }));
    assert_eq!(text.json()["payload"], json!({ "text": "hi" }));
}

#[test]
fn an_unknown_unit_is_a_404_and_a_bad_id_a_400() {
    let (_dir, server) = start();
    let missing = server.get(&format!("/v1/units/{}", "00".repeat(16)));
    assert_eq!(missing.status, 404);
    assert_eq!(error_code(&missing), "not_found");
    for bad in ["abc", &"zz".repeat(16), &"00".repeat(17)] {
        let reply = server.get(&format!("/v1/units/{bad}"));
        assert_eq!((reply.status, error_code(&reply).as_str()), (400, "invalid_request"), "{bad}");
    }
}

#[test]
fn every_failure_has_the_one_error_body() {
    let (_dir, server) = start();
    let cases = [
        (server.get("/v1/nope"), 404, "not_found"),
        (server.request("PATCH", "/v1/units", None), 405, "method_not_allowed"),
        (server.request("POST", "/v1/units", Some(&json!({ "type": "note" }))), 400, "invalid_request"),
        (
            server.request("POST", "/v1/units", Some(&json!({ "type": "", "payload": { "text": "x" } }))),
            400,
            "invalid_request",
        ),
        (server.get("/v1/units?limit=0"), 400, "invalid_request"),
        (server.get("/v1/units?bogus=1"), 400, "invalid_request"),
    ];
    for (reply, status, code) in cases {
        assert_eq!(
            (reply.status, error_code(&reply).as_str()),
            (status, code),
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        assert_eq!(reply.header("content-type"), Some("application/json"));
        assert!(reply.json()["error"]["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
}

#[test]
fn a_wrong_method_on_any_route_is_a_405_that_names_the_allowed_ones() {
    let (_dir, server) = start();
    let id = "00".repeat(16);
    let routes = [
        ("POST", "/v1/health".to_owned(), "GET"),
        ("PATCH", "/v1/units".to_owned(), "GET, POST"),
        ("POST", format!("/v1/units/{id}"), "DELETE, GET, PUT"),
        ("GET", format!("/v1/units/{id}/archive"), "POST"),
        ("GET", format!("/v1/units/{id}/unarchive"), "POST"),
        ("POST", format!("/v1/units/{id}/relations"), "GET"),
        ("GET", "/v1/relations".to_owned(), "POST"),
        ("GET", format!("/v1/relations/{id}"), "DELETE"),
        ("GET", "/v1/schemas".to_owned(), "POST"),
        ("POST", "/v1/schemas/user".to_owned(), "GET"),
        ("GET", "/v1/query".to_owned(), "POST"),
        ("GET", "/v1/query/explain".to_owned(), "POST"),
        ("GET", "/v1/search".to_owned(), "POST"),
    ];
    for (method, path, allow) in routes {
        let reply = server.request(method, &path, None);
        assert_eq!((reply.status, error_code(&reply).as_str()), (405, "method_not_allowed"), "{method} {path}");
        assert_eq!(reply.header("allow"), Some(allow), "{method} {path}");
    }
}

#[test]
fn a_misspelled_query_parameter_is_refused_not_ignored() {
    let (_dir, server) = start();
    let id = note(&server, "x");
    assert_eq!(
        server.put(&format!("/v1/units/{id}?expected_rev=1"), &json!({ "payload": { "text": "y" } })).status,
        400
    );
    assert_eq!(server.request("POST", &format!("/v1/units/{id}/archive?rev=1"), None).status, 400);
    assert_eq!(server.get(&format!("/v1/units/{id}/relations?dir=out")).status, 400);
    assert_eq!(server.get(&format!("/v1/units/{id}")).json()["revision"], 1, "nothing was changed");
}

#[test]
fn a_body_that_is_not_json_names_the_problem() {
    let (_dir, server) = start();
    let reply = raw_request(server.http, "POST", "/v1/units", Some(b"{not json"));
    assert_eq!(reply.status, 400);
    assert!(reply.json()["error"]["message"].as_str().unwrap().contains("invalid request body"));
}

#[test]
fn a_body_over_the_limit_is_a_413_with_the_error_body() {
    let (_dir, server) = start();
    let big = vec![b'a'; kerno_server::MAX_BODY + 1];
    let reply = raw_request(server.http, "POST", "/v1/units", Some(&big));
    assert_eq!((reply.status, error_code(&reply).as_str()), (413, "payload_too_large"));
}

#[test]
fn a_payload_over_the_engine_limit_is_a_413() {
    let (_dir, server) = start();
    let big = "a".repeat(2 << 20);
    let reply = server.post("/v1/units", &json!({ "type": "note", "payload": { "text": big } }));
    assert_eq!((reply.status, error_code(&reply).as_str()), (413, "payload_too_large"));
}

#[test]
fn units_survive_a_graceful_stop_and_the_directory_is_free_again() {
    let dir = TempDir::new();
    let server = Server::start(&dir.db());
    let id = note(&server, "durable");
    let status = server.stop();
    assert!(status.success(), "the server must exit cleanly on SIGTERM: {status}");

    let again = Server::start(&dir.db());
    let read = again.get(&format!("/v1/units/{id}"));
    assert_eq!(read.status, 200);
    assert_eq!(read.json()["payload"], json!({ "text": "durable" }));
}

#[test]
fn a_second_server_on_the_same_directory_refuses_to_start() {
    let (dir, _server) = start();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kerno-server"))
        .env_remove("KERNO_SERVER_DB")
        .arg("--db")
        .arg(dir.db())
        .args(["--listen", "127.0.0.1:0"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot open the database"), "{stderr}");
}

#[test]
fn the_binary_needs_a_database_and_says_how_to_give_one() {
    let out =
        std::process::Command::new(env!("CARGO_BIN_EXE_kerno-server")).env_remove("KERNO_SERVER_DB").output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no database") && stderr.contains("usage:"), "{stderr}");
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_kerno-server")).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("usage:"));
}

#[test]
fn the_database_can_come_from_the_environment() {
    let dir = TempDir::new();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_kerno-server"))
        .env("KERNO_SERVER_DB", dir.db())
        .args(["--listen", "127.0.0.1:0"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.take().unwrap()), &mut line).unwrap();
    let up = line.starts_with("listening on http://");
    let _ = child.kill();
    let _ = child.wait();
    assert!(up, "{line}");
    // The database was created where the environment said.
    assert!(std::fs::read_dir(dir.db()).unwrap().count() > 0);
}

// ------------------------------------------------------------ slice 2: CRUD

fn relate(server: &Server, from: &str, to: &str, relation_type: &str) -> String {
    let reply = server.post("/v1/relations", &json!({ "from": from, "to": to, "type": relation_type }));
    assert_eq!(reply.status, 201, "{}", String::from_utf8_lossy(&reply.body));
    reply.json()["id"].as_str().unwrap().to_owned()
}

fn ids(list: &Value, key: &str) -> Vec<String> {
    list[key].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn units_are_listed_by_type_in_id_order_and_paged() {
    let (_dir, server) = start();
    let mut notes: Vec<String> = (0..5).map(|i| note(&server, &format!("n{i}"))).collect();
    let other = server.post("/v1/units", &json!({ "type": "task", "payload": { "text": "t" } })).json();
    notes.sort();

    let all = server.get("/v1/units?type=note").json();
    assert_eq!(ids(&all, "units"), notes);
    assert_eq!(all["next"], Value::Null);
    assert_eq!(ids(&server.get("/v1/units?type=task").json(), "units"), [other["id"].as_str().unwrap()]);
    assert_eq!(server.get("/v1/units").json()["units"].as_array().unwrap().len(), 6);

    let first = server.get("/v1/units?type=note&limit=2").json();
    assert_eq!(ids(&first, "units"), notes[..2]);
    assert_eq!(first["next"], json!(notes[1]));
    let second = server.get(&format!("/v1/units?type=note&limit=2&after={}", notes[1])).json();
    assert_eq!(ids(&second, "units"), notes[2..4]);
    let last = server.get(&format!("/v1/units?type=note&limit=2&after={}", notes[3])).json();
    assert_eq!(ids(&last, "units"), notes[4..]);
    assert_eq!(last["next"], Value::Null);
    // A page that is exactly the rest has no next.
    let exact = server.get(&format!("/v1/units?type=note&limit=1&after={}", notes[3])).json();
    assert_eq!((ids(&exact, "units"), exact["next"].clone()), (notes[4..].to_vec(), Value::Null));
    assert_eq!(server.get("/v1/units?limit=1001").status, 400);
    assert_eq!(server.get("/v1/units?after=xyz").status, 400);
}

#[test]
fn a_page_holds_a_hundred_units_unless_asked_for_more_up_to_a_thousand() {
    let (_dir, server) = start();
    for i in 0..101 {
        note(&server, &format!("n{i}"));
    }
    let first = server.get("/v1/units?type=note").json();
    assert_eq!(first["units"].as_array().unwrap().len(), 100);
    assert!(first["next"].is_string(), "the hundred-and-first unit is on the next page");
    let all = server.get("/v1/units?type=note&limit=1000");
    assert_eq!(all.status, 200);
    let all = all.json();
    assert_eq!((all["units"].as_array().unwrap().len(), all["next"].clone()), (101, Value::Null));
    assert_eq!(server.get("/v1/units?limit=1").json()["units"].as_array().unwrap().len(), 1);
}

#[test]
fn an_update_makes_a_new_revision() {
    let (_dir, server) = start();
    let id = note(&server, "first");
    let updated = server.put(&format!("/v1/units/{id}"), &json!({ "payload": { "text": "second" } }));
    assert_eq!(updated.status, 200);
    let unit = updated.json();
    assert_eq!((unit["revision"].clone(), unit["payload"].clone()), (json!(2), json!({ "text": "second" })));
    assert!(unit["updated_at_us"].as_i64() >= unit["created_at_us"].as_i64());
    assert_eq!(server.get(&format!("/v1/units/{id}")).json(), unit);

    let missing = server.put(&format!("/v1/units/{}", "00".repeat(16)), &json!({ "payload": { "text": "x" } }));
    assert_eq!(missing.status, 404);
}

#[test]
fn expected_revision_guards_every_change() {
    let (_dir, server) = start();
    let id = note(&server, "v1");
    let path = format!("/v1/units/{id}");
    let body = json!({ "payload": { "text": "v2" } });

    let stale = server.put(&format!("{path}?expected_revision=7"), &body);
    assert_eq!((stale.status, error_code(&stale).as_str()), (409, "revision_conflict"));
    assert_eq!(server.get(&path).json()["revision"], 1, "a refused update changes nothing");
    assert_eq!(server.put(&format!("{path}?expected_revision=1"), &body).status, 200);

    // The same guard on archive, unarchive and delete.
    for op in ["archive", "unarchive"] {
        let stale = server.request("POST", &format!("{path}/{op}?expected_revision=1"), None);
        assert_eq!((stale.status, error_code(&stale).as_str()), (409, "revision_conflict"), "{op}");
    }
    assert_eq!(server.request("POST", &format!("{path}/archive?expected_revision=2"), None).status, 200);
    assert_eq!(server.request("POST", &format!("{path}/unarchive?expected_revision=3"), None).status, 200);
    let stale = server.delete(&format!("{path}?expected_revision=1"));
    assert_eq!((stale.status, error_code(&stale).as_str()), (409, "revision_conflict"));
    assert_eq!(server.delete(&format!("{path}?expected_revision=4")).status, 204);
    assert_eq!(server.put(&format!("{path}?expected_revision=x"), &body).status, 400);
}

#[test]
fn an_archived_unit_leaves_the_list_and_comes_back() {
    let (_dir, server) = start();
    let (keep, shelve) = (note(&server, "keep"), note(&server, "shelve"));
    let archived = server.request("POST", &format!("/v1/units/{shelve}/archive"), None);
    assert_eq!(archived.status, 200);
    assert_eq!(archived.json()["lifecycle"], "archived");
    assert_eq!(archived.json()["revision"], 2);
    assert_eq!(ids(&server.get("/v1/units?type=note").json(), "units"), std::slice::from_ref(&keep));
    // Still readable by id, payload intact.
    assert_eq!(server.get(&format!("/v1/units/{shelve}")).json()["payload"], json!({ "text": "shelve" }));

    // An archived unit cannot be archived again.
    let again = server.request("POST", &format!("/v1/units/{shelve}/archive"), None);
    assert_eq!((again.status, error_code(&again).as_str()), (400, "invalid_request"));
    let update = server.put(&format!("/v1/units/{shelve}"), &json!({ "payload": { "text": "x" } }));
    assert_eq!(update.status, 400, "an archived unit takes no update");

    let back = server.request("POST", &format!("/v1/units/{shelve}/unarchive"), None);
    assert_eq!((back.status, back.json()["lifecycle"].clone()), (200, json!("active")));
    assert_eq!(server.get("/v1/units?type=note").json()["units"].as_array().unwrap().len(), 2);
    let not_archived = server.request("POST", &format!("/v1/units/{keep}/unarchive"), None);
    assert_eq!((not_archived.status, error_code(&not_archived).as_str()), (400, "invalid_request"));
}

#[test]
fn a_deleted_unit_stays_as_a_tombstone() {
    let (_dir, server) = start();
    let id = note(&server, "gone soon");
    let path = format!("/v1/units/{id}");
    let deleted = server.delete(&path);
    assert_eq!((deleted.status, deleted.body.len()), (204, 0));
    let tombstone = server.get(&path).json();
    assert_eq!((tombstone["lifecycle"].clone(), tombstone["payload"].clone()), (json!("deleted"), Value::Null));
    assert_eq!(server.get("/v1/units?type=note").json()["units"].as_array().unwrap().len(), 0);
    // A deleted unit takes no more changes.
    let update = server.put(&path, &json!({ "payload": { "text": "x" } }));
    assert_eq!((update.status, error_code(&update).as_str()), (409, "conflict"));
    assert_eq!(server.delete(&path).status, 409);
    assert_eq!(server.delete(&format!("/v1/units/{}", "00".repeat(16))).status, 404);
}

// -------------------------------------------------------- slice 2: relations

#[test]
fn a_relation_is_created_and_listed_from_both_ends() {
    let (_dir, server) = start();
    let (a, b) = (note(&server, "a"), note(&server, "b"));
    let created = server.post("/v1/relations", &json!({ "from": a, "to": b, "type": "about" }));
    assert_eq!(created.status, 201);
    let edge = created.json();
    assert_eq!(edge["from"], json!(a));
    assert_eq!(edge["to"], json!(b));
    assert_eq!((edge["type"].clone(), edge["provenance"].clone()), (json!("about"), json!("declared")));
    assert!(edge.get("direction").is_none(), "a created relation has no direction");

    let out = server.get(&format!("/v1/units/{a}/relations?direction=out")).json();
    assert_eq!(out["relations"].as_array().unwrap().len(), 1);
    assert_eq!(
        (out["relations"][0]["id"].clone(), out["relations"][0]["direction"].clone()),
        (edge["id"].clone(), json!("out"))
    );
    // Nothing starts at `b`, so its outgoing list is empty and its incoming one is not.
    assert_eq!(server.get(&format!("/v1/units/{b}/relations?direction=out")).json()["relations"], json!([]));
    let incoming = server.get(&format!("/v1/units/{b}/relations?direction=in")).json();
    assert_eq!(
        (incoming["relations"][0]["id"].clone(), incoming["relations"][0]["direction"].clone()),
        (edge["id"].clone(), json!("in"))
    );
    assert_eq!(server.get(&format!("/v1/units/{a}/relations?direction=in")).json()["relations"], json!([]));
}

#[test]
fn a_relation_written_by_other_means_keeps_its_provenance() {
    use kerno::units::{ChangeMeta, RelationProvenance};
    let dir = TempDir::new();
    // The API only creates declared relations, so the others are written to the database first.
    let db = kerno::Db::open(dir.db()).unwrap();
    let (a, b) = (db.store("note", "a").unwrap(), db.store("note", "b").unwrap());
    let mut changes = db.unit_db().begin_changeset(ChangeMeta::default()).unwrap();
    let declared = RelationProvenance::Declared;
    let imported = RelationProvenance::Imported { source: b"wiki".to_vec() };
    let algorithmic = RelationProvenance::Algorithmic { origin: b"tfidf".to_vec() };
    let inferred = RelationProvenance::Inferred { origin: b"model".to_vec(), confidence_permille: 700 };
    for (relation_type, provenance) in [("d", declared), ("i", imported), ("a", algorithmic), ("f", inferred)] {
        changes.create_relation(a, b, relation_type, provenance).unwrap();
    }
    changes.commit().unwrap();
    db.close().unwrap();

    let server = Server::start(&dir.db());
    let listed = server.get(&format!("/v1/units/{a}/relations?direction=out")).json();
    let provenance: std::collections::BTreeMap<String, String> = listed["relations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["type"].as_str().unwrap().to_owned(), r["provenance"].as_str().unwrap().to_owned()))
        .collect();
    let expected = [("d", "declared"), ("i", "imported"), ("a", "algorithmic"), ("f", "inferred")];
    assert_eq!(provenance, expected.iter().map(|(t, p)| ((*t).to_owned(), (*p).to_owned())).collect());
}

#[test]
fn listing_relations_defaults_to_both_directions() {
    let (_dir, server) = start();
    let (a, b, c) = (note(&server, "a"), note(&server, "b"), note(&server, "c"));
    let outgoing = relate(&server, &b, &c, "about");
    let incoming = relate(&server, &a, &b, "about");
    let both = server.get(&format!("/v1/units/{b}/relations")).json();
    let found: Vec<(String, String)> = both["relations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap().to_owned(), r["direction"].as_str().unwrap().to_owned()))
        .collect();
    assert_eq!(found, [(outgoing, "out".to_owned()), (incoming, "in".to_owned())]);
    assert_eq!(server.get(&format!("/v1/units/{b}/relations?direction=both")).json(), both);
    assert_eq!(server.get(&format!("/v1/units/{b}/relations?direction=sideways")).status, 400);
}

#[test]
fn relations_can_be_filtered_by_type() {
    let (_dir, server) = start();
    let (a, b) = (note(&server, "a"), note(&server, "b"));
    let about = relate(&server, &a, &b, "about");
    relate(&server, &a, &b, "cites");
    let only = server.get(&format!("/v1/units/{a}/relations?type=about")).json();
    assert_eq!(ids(&only, "relations"), [about]);
    assert_eq!(server.get(&format!("/v1/units/{a}/relations")).json()["relations"].as_array().unwrap().len(), 2);
    assert_eq!(server.get(&format!("/v1/units/{a}/relations?type=none")).json()["relations"], json!([]));
}

#[test]
fn a_unit_related_to_itself_lists_the_relation_on_both_sides() {
    let (_dir, server) = start();
    let a = note(&server, "a");
    relate(&server, &a, &a, "about");
    let both = server.get(&format!("/v1/units/{a}/relations")).json();
    let directions: Vec<&str> =
        both["relations"].as_array().unwrap().iter().map(|r| r["direction"].as_str().unwrap()).collect();
    assert_eq!(directions, ["out", "in"]);
}

#[test]
fn relation_errors_are_told_apart() {
    let (_dir, server) = start();
    let a = note(&server, "a");
    let nobody = "00".repeat(16);
    let to_nobody = server.post("/v1/relations", &json!({ "from": a, "to": nobody, "type": "about" }));
    assert_eq!((to_nobody.status, error_code(&to_nobody).as_str()), (404, "not_found"));
    let bad_type = server.post("/v1/relations", &json!({ "from": a, "to": a, "type": "" }));
    assert_eq!((bad_type.status, error_code(&bad_type).as_str()), (400, "invalid_request"));
    let bad_id = server.post("/v1/relations", &json!({ "from": "x", "to": a, "type": "about" }));
    assert_eq!(bad_id.status, 400);
    assert_eq!(server.get(&format!("/v1/units/{nobody}/relations")).status, 404);
    assert_eq!(server.delete(&format!("/v1/relations/{nobody}")).status, 404);
    assert_eq!(server.delete("/v1/relations/zz").status, 400);
}

#[test]
fn deleting_a_relation_removes_it_from_both_ends() {
    let (_dir, server) = start();
    let (a, b) = (note(&server, "a"), note(&server, "b"));
    let edge = relate(&server, &a, &b, "about");
    let deleted = server.delete(&format!("/v1/relations/{edge}"));
    assert_eq!((deleted.status, deleted.body.len()), (204, 0));
    assert_eq!(server.get(&format!("/v1/units/{a}/relations")).json()["relations"], json!([]));
    assert_eq!(server.get(&format!("/v1/units/{b}/relations")).json()["relations"], json!([]));
    assert_eq!(server.delete(&format!("/v1/relations/{edge}")).status, 404);
}

#[test]
fn a_unit_with_relations_cannot_be_deleted_from_either_end() {
    let (_dir, server) = start();
    let (a, b) = (note(&server, "a"), note(&server, "b"));
    let edge = relate(&server, &a, &b, "about");
    // `a` only has an outgoing relation, `b` only an incoming one: both are refused.
    for unit in [&a, &b] {
        let refused = server.delete(&format!("/v1/units/{unit}"));
        assert_eq!((refused.status, error_code(&refused).as_str()), (409, "conflict"), "{unit}");
        let refused = server.request("POST", &format!("/v1/units/{unit}/archive"), None);
        assert_eq!((refused.status, error_code(&refused).as_str()), (409, "conflict"), "{unit}");
    }
    assert_eq!(server.delete(&format!("/v1/relations/{edge}")).status, 204);
    for unit in [&a, &b] {
        assert_eq!(server.delete(&format!("/v1/units/{unit}")).status, 204, "{unit}");
    }
}

// ------------------------------------------------- slice 3: schemas and queries

fn define_user_schema(server: &Server) {
    let reply = server.post(
        "/v1/schemas",
        &json!({ "type": "user", "fields": [
            { "name": "email", "type": "text", "unique": true, "required": true },
            { "name": "age", "type": "int", "indexed": true },
            { "name": "score", "type": "float" },
            { "name": "avatar", "type": "bytes" },
            { "name": "admin", "type": "bool" },
        ] }),
    );
    assert_eq!(reply.status, 201, "{}", String::from_utf8_lossy(&reply.body));
}

fn user(server: &Server, email: &str, age: i64) -> String {
    let reply =
        server.post("/v1/units", &json!({ "type": "user", "payload": { "fields": { "email": email, "age": age } } }));
    assert_eq!(reply.status, 201, "{}", String::from_utf8_lossy(&reply.body));
    reply.json()["id"].as_str().unwrap().to_owned()
}

#[test]
fn a_schema_is_defined_and_read_back_with_its_flags() {
    let (_dir, server) = start();
    define_user_schema(&server);
    let schema = server.get("/v1/schemas/user");
    assert_eq!(schema.status, 200);
    // Every field, in name order, with its type and all three flags.
    let field = |name: &str, ty: &str, required: bool, unique: bool, indexed: bool| json!({ "name": name, "type": ty, "required": required, "unique": unique, "indexed": indexed });
    assert_eq!(
        schema.json()["fields"],
        json!([
            field("admin", "bool", false, false, false),
            field("age", "int", false, false, true),
            field("avatar", "bytes", false, false, false),
            field("email", "text", true, true, true),
            field("score", "float", false, false, false),
        ])
    );
}

#[test]
fn schema_errors_are_told_apart() {
    let (_dir, server) = start();
    define_user_schema(&server);
    let again = server.post("/v1/schemas", &json!({ "type": "user", "fields": [{ "name": "x", "type": "int" }] }));
    assert_eq!((again.status, error_code(&again).as_str()), (400, "invalid_request"));
    let unknown_type =
        server.post("/v1/schemas", &json!({ "type": "t", "fields": [{ "name": "x", "type": "decimal" }] }));
    assert_eq!(unknown_type.status, 400);
    let twice = server.post(
        "/v1/schemas",
        &json!({ "type": "t", "fields": [{ "name": "x", "type": "int" }, { "name": "x", "type": "int" }] }),
    );
    assert_eq!(twice.status, 400);
    let missing = server.get("/v1/schemas/nothing");
    assert_eq!((missing.status, error_code(&missing).as_str()), (404, "not_found"));
    // A unit type is any UTF-8: it travels percent-encoded in the path.
    let spaced = server.post("/v1/schemas", &json!({ "type": "my type", "fields": [{ "name": "x", "type": "int" }] }));
    assert_eq!(spaced.status, 201);
    assert_eq!(server.get("/v1/schemas/my%20type").json()["type"], "my type");
}

#[test]
fn a_typed_unit_holds_its_fields_of_every_type() {
    let (_dir, server) = start();
    define_user_schema(&server);
    let fields = json!({ "email": "ada@x", "age": 36, "score": 4.5, "avatar": { "hex": "00ff" }, "admin": true });
    let created = server.post("/v1/units", &json!({ "type": "user", "payload": { "fields": fields } }));
    assert_eq!(created.status, 201);
    assert_eq!(created.json()["payload"], json!({ "fields": fields }));
    let id = created.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(server.get(&format!("/v1/units/{id}")).json()["payload"], json!({ "fields": fields }));
    // The list reads the same units as fields too.
    assert_eq!(server.get("/v1/units?type=user").json()["units"][0]["payload"], json!({ "fields": fields }));
}

#[test]
fn an_integer_written_for_a_float_field_is_a_float() {
    let (_dir, server) = start();
    define_user_schema(&server);
    // JavaScript cannot write `5.0`: the schema says the field is a float.
    let created =
        server.post("/v1/units", &json!({ "type": "user", "payload": { "fields": { "email": "a@x", "score": 5 } } }));
    assert_eq!(created.status, 201);
    assert_eq!(serde_json::to_string(&created.json()["payload"]["fields"]["score"]).unwrap(), "5.0");
    // The other way is not forgiven: a float for an int field.
    let refused =
        server.post("/v1/units", &json!({ "type": "user", "payload": { "fields": { "email": "b@x", "age": 5.5 } } }));
    assert_eq!((refused.status, error_code(&refused).as_str()), (422, "schema_violation"));
}

#[test]
fn an_integer_becomes_a_float_only_when_the_float_is_exactly_that_integer() {
    let (_dir, server) = start();
    define_user_schema(&server);
    let score = |n: i64| {
        server.post(
            "/v1/units",
            &json!({ "type": "user", "payload": { "fields": { "email": format!("{n}@x"), "score": n } } }),
        )
    };
    assert_eq!(score(1 << 53).status, 201);
    assert_eq!(score(-(1 << 53)).status, 201);
    // 2^53 + 1 is not a double: it is not turned into a nearby float.
    let refused = score((1 << 53) + 1);
    assert_eq!((refused.status, error_code(&refused).as_str()), (422, "schema_violation"));
    assert_eq!(score(i64::MAX).status, 422);
}

#[test]
fn the_schema_refuses_what_does_not_fit_and_nothing_is_stored() {
    let (_dir, server) = start();
    define_user_schema(&server);
    user(&server, "ada@x", 36);
    let cases = [
        json!({ "fields": { "age": 3 } }),
        json!({ "fields": { "email": "b@x", "age": "old" } }),
        json!({ "fields": { "email": "b@x", "zzz": 1 } }),
        json!({ "fields": { "email": "ada@x" } }),
        json!({ "text": "not a record" }),
    ];
    for payload in cases {
        let reply = server.post("/v1/units", &json!({ "type": "user", "payload": payload }));
        assert_eq!((reply.status, error_code(&reply).as_str()), (422, "schema_violation"), "{payload}");
    }
    assert_eq!(server.get("/v1/units?type=user").json()["units"].as_array().unwrap().len(), 1);
    // Fields for a type without a schema have nothing to be checked against.
    let free = server.post("/v1/units", &json!({ "type": "free", "payload": { "fields": { "a": 1 } } }));
    assert_eq!((free.status, error_code(&free).as_str()), (400, "invalid_request"));
}

#[test]
fn a_typed_unit_is_updated_through_its_schema() {
    let (_dir, server) = start();
    define_user_schema(&server);
    let id = user(&server, "ada@x", 36);
    let other = user(&server, "bob@x", 29);
    let path = format!("/v1/units/{id}");
    let ok = server.put(&path, &json!({ "payload": { "fields": { "email": "ada@x", "age": 37 } } }));
    assert_eq!(
        (ok.status, ok.json()["payload"]["fields"]["age"].clone(), ok.json()["revision"].clone()),
        (200, json!(37), json!(2))
    );
    // A unit keeps holding its own unique value; another unit's value is refused.
    let taken = server.put(&path, &json!({ "payload": { "fields": { "email": "bob@x" } } }));
    assert_eq!((taken.status, error_code(&taken).as_str()), (422, "schema_violation"));
    assert_eq!(server.get(&format!("/v1/units/{other}")).json()["revision"], 1);
    assert_eq!(server.get(&path).json()["revision"], 2);
}

fn rows(reply: &common::Reply) -> Vec<Vec<Value>> {
    reply.json()["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().clone()).collect()
}

fn ages(rows: &[Vec<Value>], column: usize) -> Vec<i64> {
    rows.iter().map(|r| r[column]["payload"]["fields"]["age"].as_i64().unwrap()).collect()
}

#[test]
fn the_query_language_filters_orders_and_pages() {
    let (_dir, server) = start();
    define_user_schema(&server);
    for (email, age) in [("a@x", 20), ("b@x", 30), ("c@x", 40), ("d@x", 50)] {
        user(&server, email, age);
    }
    let query = |q: &str| server.post("/v1/query", &json!({ "query": q }));
    let all = query("FIND user AS u ORDER BY u.age DESC");
    assert_eq!(all.status, 200);
    assert_eq!(all.json()["aliases"], json!(["u"]));
    assert_eq!(ages(&rows(&all), 0), [50, 40, 30, 20]);
    assert_eq!(ages(&rows(&query("FIND user AS u WHERE u.age >= 30 AND u.age < 50 ORDER BY u.age")), 0), [30, 40]);
    assert_eq!(ages(&rows(&query("FIND user AS u ORDER BY u.age LIMIT 2 OFFSET 1")), 0), [30, 40]);
    assert_eq!(rows(&query("FIND user AS u WHERE u.email = \"zz\"")).len(), 0);
}

#[test]
fn a_query_joins_units_through_relations() {
    let (_dir, server) = start();
    define_user_schema(&server);
    let team = server.post(
        "/v1/schemas",
        &json!({ "type": "team", "fields": [{ "name": "name", "type": "text", "unique": true }] }),
    );
    assert_eq!(team.status, 201);
    let core = server.post("/v1/units", &json!({ "type": "team", "payload": { "fields": { "name": "core" } } })).json();
    let core = core["id"].as_str().unwrap().to_owned();
    let (ada, bob) = (user(&server, "ada@x", 36), user(&server, "bob@x", 29));
    relate(&server, &ada, &core, "member_of");
    // Bob is not in the team: no relation, no row.
    let reply = server.post(
        "/v1/query",
        &json!({ "query": "FIND user AS u JOIN team AS t VIA member_of WHERE t.name = \"core\" ORDER BY u.age" }),
    );
    assert_eq!(reply.status, 200, "{}", String::from_utf8_lossy(&reply.body));
    assert_eq!(reply.json()["aliases"], json!(["u", "t"]));
    let found = rows(&reply);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0][0]["id"].clone(), found[0][1]["id"].clone()), (json!(ada), json!(core)));
    assert_eq!(found[0][1]["payload"], json!({ "fields": { "name": "core" } }));
    assert_ne!(found[0][0]["id"], json!(bob));
    // The join read the other way.
    let reversed = server.post("/v1/query", &json!({ "query": "FIND team AS t JOIN user AS u VIA member_of IN" }));
    assert_eq!(rows(&reversed).len(), 1);
}

#[test]
fn a_query_reports_the_access_path_it_read_through() {
    let (_dir, server) = start();
    define_user_schema(&server);
    for i in 0..8 {
        user(&server, &format!("u{i}@x"), 20 + i * 3);
    }
    let indexed = server.post("/v1/query", &json!({ "query": "FIND user AS u WHERE u.age = 29" }));
    assert_eq!(indexed.json()["access"], json!({ "kind": "field_index", "field": "age" }));
    let scan = server.post("/v1/query", &json!({ "query": "FIND user AS u WHERE u.score IS NULL" }));
    assert_eq!(scan.json()["access"], json!({ "kind": "type_scan" }));
}

#[test]
fn explain_lists_the_candidates_and_the_one_chosen() {
    let (_dir, server) = start();
    define_user_schema(&server);
    for i in 0..8 {
        user(&server, &format!("u{i}@x"), 20 + i * 3);
    }
    let plan = server.post("/v1/query/explain", &json!({ "query": "FIND user AS u WHERE u.age = 29" }));
    assert_eq!(plan.status, 200);
    let plan = plan.json();
    assert_eq!((plan["root_alias"].clone(), plan["root_type"].clone()), (json!("u"), json!("user")));
    let candidates = plan["candidates"].as_array().unwrap();
    assert_eq!(candidates[0]["access"], json!({ "kind": "type_scan" }));
    assert_eq!(candidates[1]["access"], json!({ "kind": "field_index", "field": "age" }));
    assert!(candidates[1]["cost"].as_u64().unwrap() < candidates[0]["cost"].as_u64().unwrap());
    assert_eq!(plan["chosen"], 1);
    assert_eq!(plan["joins"], json!([]));
    // A query with no index to use is planned as a scan, whatever it is asked.
    let scan = server.post("/v1/query/explain", &json!({ "query": "FIND user AS u" })).json();
    assert_eq!((scan["chosen"].clone(), scan["candidates"].as_array().unwrap().len()), (json!(0), 1));
}

#[test]
fn explain_describes_the_joins_with_their_direction() {
    let (_dir, server) = start();
    define_user_schema(&server);
    server.post("/v1/schemas", &json!({ "type": "team", "fields": [{ "name": "name", "type": "text" }] }));
    let plan = server
        .post(
            "/v1/query/explain",
            &json!({ "query": "FIND user AS u JOIN team AS t VIA member_of JOIN user AS v VIA member_of IN FROM t" }),
        )
        .json();
    assert_eq!(
        plan["joins"],
        json!([
            { "alias": "t", "unit_type": "team", "relation_type": "member_of", "direction": "out", "from": "u" },
            { "alias": "v", "unit_type": "user", "relation_type": "member_of", "direction": "in", "from": "t" },
        ])
    );
}

#[test]
fn a_query_that_does_not_parse_or_fit_is_a_400_that_says_why() {
    let (_dir, server) = start();
    define_user_schema(&server);
    for (query, needle) in [
        ("FIND", "byte 4"),
        ("FIND user WHERE nope = 1", "declares no such field"),
        ("FIND user WHERE age = \"x\"", "age"),
        ("FIND ghost WHERE a = 1", "schema"),
    ] {
        for path in ["/v1/query", "/v1/query/explain"] {
            let reply = server.post(path, &json!({ "query": query }));
            assert_eq!((reply.status, error_code(&reply).as_str()), (400, "invalid_request"), "{path} {query}");
            let message = reply.json()["error"]["message"].as_str().unwrap().to_owned();
            assert!(message.contains(needle), "{query}: {message}");
        }
    }
    assert_eq!(server.post("/v1/query", &json!({})).status, 400);
    assert_eq!(server.get("/v1/query").status, 405);
}

// -------------------------------------------------------- slice 4: hybrid search

fn search(server: &Server, body: &Value) -> Value {
    let reply = server.post("/v1/search", body);
    assert_eq!(reply.status, 200, "{}", String::from_utf8_lossy(&reply.body));
    reply.json()
}

fn hit_ids(reply: &Value) -> Vec<String> {
    reply["hits"].as_array().unwrap().iter().map(|h| h["id"].as_str().unwrap().to_owned()).collect()
}

fn hit<'a>(reply: &'a Value, id: &str) -> &'a Value {
    reply["hits"].as_array().unwrap().iter().find(|h| h["id"] == id).unwrap_or_else(|| panic!("{id} is not a hit"))
}

/// What one unit earns at rank `rank` in a leg of weight `weight` at `rrf_k` (the arithmetic of
/// `kerno_units::hybrid`).
fn earns(weight: u64, rrf_k: u64, rank: u64) -> u64 {
    weight * 1_000_000_000 / (rrf_k + rank)
}

#[test]
fn the_keyword_leg_ranks_units_by_their_text() {
    let (_dir, server) = start();
    let rust = note(&server, "rust rust rust ownership");
    let mixed = note(&server, "rust and python");
    note(&server, "python only");
    let reply = search(&server, &json!({ "text": "rust" }));
    assert_eq!(hit_ids(&reply), [rust.clone(), mixed.clone()]);
    let first = hit(&reply, &rust);
    assert_eq!(first["ranks"], json!({ "keyword": 1, "vector": null, "graph": null }));
    assert_eq!(first["score"], earns(1, 60, 1), "rank 1 at the default k of 60 and weight 1");
    assert_eq!(hit(&reply, &mixed)["score"], earns(1, 60, 2));
    // The hit carries the unit.
    assert_eq!(first["unit"]["payload"], json!({ "text": "rust rust rust ownership" }));
    assert_eq!(first["unit"]["id"], json!(rust));
}

#[test]
fn a_search_returns_at_most_ten_hits_unless_asked_for_more() {
    let (_dir, server) = start();
    for i in 0..12 {
        note(&server, &format!("common word {i}"));
    }
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "common" }))).len(), 10);
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "common", "limit": 3 }))).len(), 3);
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "common", "limit": 50 }))).len(), 12);
}

#[test]
fn each_leg_contributes_at_most_a_hundred_candidates_unless_asked_for_fewer() {
    let (_dir, server) = start();
    for i in 0..101 {
        note(&server, &format!("bulk {i}"));
    }
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "bulk", "limit": 1000 }))).len(), 100);
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "bulk", "limit": 1000, "candidates": 7 }))).len(), 7);
}

#[test]
fn the_graph_leg_follows_relations_from_its_seeds() {
    let (_dir, server) = start();
    let (a, b, c, d) = (note(&server, "a"), note(&server, "b"), note(&server, "c"), note(&server, "d"));
    relate(&server, &a, &b, "next");
    relate(&server, &b, &c, "next");
    relate(&server, &c, &d, "next");
    // Two hops by default: `d` is the third.
    let reply = search(&server, &json!({ "graph": { "seeds": [a] } }));
    assert_eq!(hit_ids(&reply), [b.clone(), c.clone()]);
    assert_eq!(hit(&reply, &b)["ranks"], json!({ "keyword": null, "vector": null, "graph": 1 }));
    assert_eq!(hit(&reply, &b)["score"], earns(1, 60, 1));
    assert_eq!(
        hit_ids(&search(&server, &json!({ "graph": { "seeds": [a], "max_depth": 3 } }))),
        [b.clone(), c.clone(), d]
    );
    assert_eq!(hit_ids(&search(&server, &json!({ "graph": { "seeds": [a], "max_depth": 1 } }))), [b]);
}

#[test]
fn the_graph_leg_follows_both_directions_unless_told_one() {
    let (_dir, server) = start();
    let (a, b, c) = (note(&server, "a"), note(&server, "b"), note(&server, "c"));
    relate(&server, &a, &b, "next");
    relate(&server, &b, &c, "next");
    let reach = |graph: Value| {
        let mut ids = hit_ids(&search(&server, &json!({ "graph": graph })));
        ids.sort();
        ids
    };
    let mut both = vec![a.clone(), c.clone()];
    both.sort();
    assert_eq!(reach(json!({ "seeds": [b] })), both);
    assert_eq!(reach(json!({ "seeds": [b], "direction": "out" })), std::slice::from_ref(&c));
    assert_eq!(reach(json!({ "seeds": [b], "direction": "in" })), std::slice::from_ref(&a));
    assert_eq!(reach(json!({ "seeds": [b], "relation_types": ["other"] })), Vec::<String>::new());
    assert_eq!(reach(json!({ "seeds": [b], "relation_types": ["next"], "direction": "out" })), [c]);
}

#[test]
fn the_graph_leg_stops_at_its_visit_budget() {
    let (_dir, server) = start();
    let seed = note(&server, "seed");
    for i in 0..5 {
        let n = note(&server, &format!("n{i}"));
        relate(&server, &seed, &n, "has");
    }
    let hits = |graph: Value| hit_ids(&search(&server, &json!({ "graph": graph }))).len();
    assert_eq!(hits(json!({ "seeds": [seed] })), 5);
    assert!(hits(json!({ "seeds": [seed], "visit_budget": 1 })) < 5);
    assert_eq!(hits(json!({ "seeds": [seed], "visit_budget": 100 })), 5);
}

#[test]
fn the_graph_leg_returns_at_most_twenty_hits_by_default() {
    let (_dir, server) = start();
    let seed = note(&server, "seed");
    for i in 0..21 {
        let n = note(&server, &format!("n{i}"));
        relate(&server, &seed, &n, "has");
    }
    assert_eq!(hit_ids(&search(&server, &json!({ "graph": { "seeds": [seed] }, "limit": 100 }))).len(), 20);
    assert_eq!(
        hit_ids(&search(&server, &json!({ "graph": { "seeds": [seed], "limit": 21 }, "limit": 100 }))).len(),
        21
    );
}

#[test]
fn the_vector_leg_ranks_the_vectors_the_request_carries() {
    let (_dir, server) = start();
    let (x, y, z) = (note(&server, "x"), note(&server, "y"), note(&server, "z"));
    let vectors = json!({ x.clone(): [1.0, 0.0], y.clone(): [0.0, 1.0], z.clone(): [0.9, 0.1] });
    let reply = search(&server, &json!({ "vector": { "query": [1.0, 0.0], "vectors": vectors } }));
    assert_eq!(hit_ids(&reply), [x.clone(), z.clone(), y.clone()]);
    assert_eq!(hit(&reply, &x)["ranks"], json!({ "keyword": null, "vector": 1, "graph": null }));
    // Cosine is the default metric and ignores length; dot ranks by it. `y` is long and 45 degrees
    // off, `z` is short and exactly on the query.
    let lengths = json!({ y.clone(): [5.0, 5.0], z.clone(): [0.9, 0.0] });
    let by_cosine = search(&server, &json!({ "vector": { "query": [1.0, 0.0], "vectors": lengths.clone() } }));
    let explicit =
        search(&server, &json!({ "vector": { "query": [1.0, 0.0], "metric": "cosine", "vectors": lengths.clone() } }));
    let by_dot = search(&server, &json!({ "vector": { "query": [1.0, 0.0], "metric": "dot", "vectors": lengths } }));
    assert_eq!(hit_ids(&by_cosine), [z.clone(), y.clone()]);
    assert_eq!(hit_ids(&explicit), hit_ids(&by_cosine));
    assert_eq!(hit_ids(&by_dot), [y, z]);
}

#[test]
fn the_legs_are_fused_by_weighted_reciprocal_rank() {
    let (_dir, server) = start();
    let both = note(&server, "rust memory");
    let keyword_only = note(&server, "rust");
    let graph_only = note(&server, "elsewhere");
    let seed = note(&server, "seed");
    relate(&server, &seed, &both, "about");
    relate(&server, &seed, &graph_only, "about");
    let body = json!({ "text": "rust", "graph": { "seeds": [seed] } });
    let reply = search(&server, &body);
    // `both` is found by the two legs; the others by one.
    assert_eq!(hit_ids(&reply)[0], both);
    let top = hit(&reply, &both);
    let (k, g) = (top["ranks"]["keyword"].as_u64().unwrap(), top["ranks"]["graph"].as_u64().unwrap());
    assert_eq!(top["score"].as_u64().unwrap(), earns(1, 60, k) + earns(1, 60, g));
    assert!(hit_ids(&reply).contains(&keyword_only) && hit_ids(&reply).contains(&graph_only));
    // The seed is not a hit of the legs that would otherwise find it.
    assert!(!hit_ids(&reply).contains(&seed));

    // Weights: 0 switches a leg off, and a weight multiplies what a rank earns.
    let keyword_off =
        search(&server, &json!({ "text": "rust", "graph": { "seeds": [seed] }, "weights": { "keyword": 0 } }));
    assert!(hit(&keyword_off, &both)["ranks"]["keyword"].is_null());
    assert!(!hit_ids(&keyword_off).contains(&keyword_only));
    let heavy = search(&server, &json!({ "text": "rust", "weights": { "keyword": 3 } }));
    assert_eq!(
        hit(&heavy, &keyword_only)["score"].as_u64().unwrap(),
        earns(3, 60, hit(&heavy, &keyword_only)["ranks"]["keyword"].as_u64().unwrap())
    );
    // A weight left out is 1, not 0.
    let partial = search(&server, &json!({ "text": "rust", "graph": { "seeds": [seed] }, "weights": { "graph": 2 } }));
    let top = hit(&partial, &both);
    assert_eq!(
        top["score"].as_u64().unwrap(),
        earns(1, 60, top["ranks"]["keyword"].as_u64().unwrap()) + earns(2, 60, top["ranks"]["graph"].as_u64().unwrap())
    );
}

#[test]
fn rrf_k_changes_what_a_rank_earns() {
    let (_dir, server) = start();
    let only = note(&server, "alone");
    let reply = search(&server, &json!({ "text": "alone", "rrf_k": 0 }));
    assert_eq!(hit(&reply, &only)["score"], earns(1, 0, 1));
    assert_eq!(earns(1, 0, 1), 1_000_000_000);
    let reply = search(&server, &json!({ "text": "alone", "rrf_k": 940 }));
    assert_eq!(hit(&reply, &only)["score"], earns(1, 940, 1));
}

#[test]
fn a_deleted_unit_is_never_a_hit() {
    let (_dir, server) = start();
    let keep = note(&server, "findable keep");
    let gone = note(&server, "findable gone");
    let vectors = json!({ keep.clone(): [1.0, 0.0], gone.clone(): [1.0, 0.0] });
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "findable" }))).len(), 2);
    assert_eq!(server.delete(&format!("/v1/units/{gone}")).status, 204);
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "findable" }))), std::slice::from_ref(&keep));
    // The vector the request still carries for it does not bring it back.
    let reply = search(&server, &json!({ "vector": { "query": [1.0, 0.0], "vectors": vectors } }));
    assert_eq!(hit_ids(&reply), [keep]);
}

#[test]
fn a_search_that_cannot_run_is_a_400_that_says_why() {
    let (_dir, server) = start();
    let a = note(&server, "a");
    let b = note(&server, "b");
    server.delete(&format!("/v1/units/{b}"));
    let cases = [
        (json!({}), "no leg"),
        (json!({ "text": "x", "weights": { "keyword": 0 } }), "no leg"),
        (json!({ "text": "x", "limit": 0 }), "limit"),
        (json!({ "text": "x", "limit": 1001 }), "limit"),
        (json!({ "text": "x", "candidates": 0 }), "candidates"),
        (json!({ "text": "x", "weights": { "keyword": 1001 } }), "weight"),
        (json!({ "graph": { "seeds": [] } }), "seed"),
        (json!({ "graph": { "seeds": [b] } }), "not an active unit"),
        (json!({ "graph": { "seeds": ["nope"] } }), "unit id"),
        (json!({ "graph": { "seeds": [a], "max_depth": 9 } }), "depth"),
        (json!({ "vector": { "query": [], "vectors": {} } }), "vector"),
        (json!({ "vector": { "query": [1.0, 0.0], "vectors": { a.clone(): [1.0] } } }), "vector"),
        (json!({ "vector": { "query": [1.0, 0.0], "vectors": { "nope": [1.0, 0.0] } } }), "unit id"),
        (json!({ "vector": { "query": [0.0, 0.0], "vectors": { a.clone(): [1.0, 0.0] } } }), "vector"),
        (json!({ "vector": { "query": [1.0], "metric": "euclid", "vectors": {} } }), "euclid"),
        (json!({ "text": "x", "surprise": 1 }), "surprise"),
    ];
    for (body, needle) in cases {
        let reply = server.post("/v1/search", &body);
        assert_eq!((reply.status, error_code(&reply).as_str()), (400, "invalid_request"), "{body}");
        let message = reply.json()["error"]["message"].as_str().unwrap().to_lowercase();
        assert!(message.contains(needle), "{body}: {message}");
    }
    assert_eq!(server.get("/v1/search").status, 405);
}

#[test]
fn the_keyword_index_of_a_type_is_turned_on_over_http() {
    // `memo` is not in the suite's --fulltext list: off by default (ADR-0023).
    let (_dir, server) = start();
    let body = json!({ "type": "memo", "payload": { "text": "orchid greenhouse" } });
    let created = server.post("/v1/units", &body);
    assert_eq!(created.status, 201);
    let id = created.json()["id"].as_str().unwrap().to_owned();
    assert!(hit_ids(&search(&server, &json!({ "text": "orchid" }))).is_empty(), "not indexed yet");
    let state = server.get("/v1/types/memo/fulltext");
    assert_eq!((state.status, state.json()), (200, json!({ "type": "memo", "enabled": false })));

    // Turning it on indexes the units the type already has, in the same commit.
    let on = server.put("/v1/types/memo/fulltext", &json!({ "enabled": true }));
    assert_eq!((on.status, on.json()), (200, json!({ "type": "memo", "enabled": true })));
    assert_eq!(hit_ids(&search(&server, &json!({ "text": "orchid" }))), [id]);

    let off = server.put("/v1/types/memo/fulltext", &json!({ "enabled": false }));
    assert_eq!(off.json()["enabled"], json!(false));
    assert!(hit_ids(&search(&server, &json!({ "text": "orchid" }))).is_empty());
    assert_eq!(server.put("/v1/types/memo/fulltext", &json!({ "on": true })).status, 400, "unknown field");
    assert_eq!(server.delete("/v1/types/memo/fulltext").status, 405);
}
