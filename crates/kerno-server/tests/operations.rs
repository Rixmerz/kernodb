//! End to end, for running the server in the open: tokens, tenants, TLS, the probes, the metrics
//! and the logs. Every test starts the real binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use common::{Reply, Server, TempDir, raw_request, tls_args, tls_file};
use kerno_server::auth::hash_hex;
use kerno_server::grpc::{AUTHORIZATION_KEY, ERROR_CODE_KEY, pb};
use pb::kerno_client::KernoClient;
use serde_json::{Value, json};
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tonic::{Code, Request};

/// Two tokens of the tenant `acme`, one of `globex`.
const ACME: &str = "acme-0000000000000000000000000000000001";
const ACME_TOO: &str = "acme-0000000000000000000000000000000002";
const GLOBEX: &str = "globex-00000000000000000000000000000001";

/// A data directory and the tokens file for [`ACME`], [`ACME_TOO`] and [`GLOBEX`].
struct Tenants {
    dir: TempDir,
}

impl Tenants {
    fn new() -> Self {
        let dir = TempDir::new();
        let file = format!(
            "# test tokens\n{} acme\n{} acme\n{} globex\n",
            hash_hex(ACME),
            hash_hex(ACME_TOO),
            hash_hex(GLOBEX)
        );
        std::fs::write(dir.0.join("tokens"), file).unwrap();
        Tenants { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.0.join("tenants")
    }

    fn log(&self) -> PathBuf {
        self.dir.0.join("server.log")
    }

    /// `--data-dir <root> --tokens <file>` and then `extra`.
    fn args(&self, extra: &[String]) -> Vec<String> {
        let mut args = vec![
            "--data-dir".to_owned(),
            self.root().to_str().unwrap().to_owned(),
            "--tokens".to_owned(),
            self.dir.0.join("tokens").to_str().unwrap().to_owned(),
        ];
        args.extend_from_slice(extra);
        args
    }

    fn start(&self, extra: &[String]) -> Server {
        let args = self.args(extra);
        Server::launch(&args.iter().map(String::as_str).collect::<Vec<_>>(), Some(&self.log()))
    }

    /// Every line the server logged, each parsed: a line that is not JSON fails the test.
    fn log_lines(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.log()).unwrap();
        text.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}"))).collect()
    }
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

fn as_user(server: &Server, token: &str, method: &str, path: &str, body: Option<&Value>) -> Reply {
    server.call(method, path, &[("Authorization", &bearer(token))], body)
}

fn create_note(server: &Server, token: &str, text: &str) -> String {
    let reply =
        as_user(server, token, "POST", "/v1/units", Some(&json!({ "type": "note", "payload": { "text": text } })));
    assert_eq!(reply.status, 201, "{}", String::from_utf8_lossy(&reply.body));
    reply.json()["id"].as_str().unwrap().to_owned()
}

fn listed(server: &Server, token: &str) -> Vec<String> {
    let reply = as_user(server, token, "GET", "/v1/units?type=note", None);
    assert_eq!(reply.status, 200);
    reply.json()["units"].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap().to_owned()).collect()
}

fn assert_unauthenticated(reply: &Reply, what: &str) {
    assert_eq!(reply.status, 401, "{what}");
    assert_eq!(reply.header("www-authenticate"), Some("Bearer"), "{what}");
    assert_eq!(
        reply.json(),
        json!({ "error": { "code": "unauthenticated",
                           "message": "a valid API token is required: send `Authorization: Bearer <token>`" } }),
        "{what}"
    );
}

fn metric(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.parse().ok())
}

// ------------------------------------------------------------------ tokens

#[test]
fn every_api_route_refuses_a_request_without_a_valid_token_the_same_way() {
    let tenants = Tenants::new();
    let server = tenants.start(&[]);
    let refused: [(&str, &[(&str, &str)]); 7] = [
        ("no header", &[]),
        ("another scheme", &[("Authorization", "Basic YWNtZTpzZWNyZXQ=")]),
        ("an empty bearer", &[("Authorization", "Bearer ")]),
        ("an unknown token", &[("Authorization", "Bearer acme-0000000000000000000000000000000009")]),
        // The tokens file holds hashes: a hash is not a token.
        ("the hash of a token", &[("Authorization", &bearer(&hash_hex(ACME)))]),
        ("a token without its scheme", &[("Authorization", ACME)]),
        ("a token in another header", &[("X-Api-Token", ACME)]),
    ];
    for (what, headers) in refused {
        assert_unauthenticated(&server.call("GET", "/v1/health", headers, None), what);
    }
    // The token is checked before the route, the method or the body is looked at.
    assert_unauthenticated(&server.call("GET", "/v1/no-such-route", &[], None), "unknown route");
    assert_unauthenticated(&server.call("PATCH", "/v1/units", &[], None), "wrong method");
    let junk = raw_request(server.http, "POST", "/v1/units", Some(b"not json"));
    assert_unauthenticated(&junk, "bad body");
    // Nothing was written by any of them.
    assert!(listed(&server, ACME).is_empty());

    // The scheme is case-insensitive, as RFC 7235 says.
    for header in [bearer(ACME), format!("bearer {ACME}"), format!("BEARER {ACME}")] {
        let reply = server.call("GET", "/v1/health", &[("Authorization", &header)], None);
        assert_eq!(reply.status, 200, "{header}");
    }
}

#[test]
fn a_server_with_one_database_needs_no_token() {
    let dir = TempDir::new();
    let server = Server::start(&dir.db());
    assert_eq!(server.get("/v1/health").status, 200);
    // A token sent anyway is ignored, not refused.
    assert_eq!(server.call("GET", "/v1/health", &[("Authorization", "Bearer whatever")], None).status, 200);
}

#[test]
fn each_token_reaches_its_own_tenant_and_no_other() {
    let tenants = Tenants::new();
    let server = tenants.start(&[]);
    let acme = create_note(&server, ACME, "acme's secret");
    let globex = create_note(&server, GLOBEX, "globex's secret");

    // Two tokens of one tenant share its database.
    let read = as_user(&server, ACME_TOO, "GET", &format!("/v1/units/{acme}"), None);
    assert_eq!((read.status, read.json()["payload"]["text"].as_str()), (200, Some("acme's secret")));
    assert_eq!(listed(&server, ACME_TOO), [acme.as_str()]);

    // Another tenant cannot read, list, find, change or delete it, even knowing its id.
    for (method, path, body) in [
        ("GET", format!("/v1/units/{acme}"), None),
        ("PUT", format!("/v1/units/{acme}"), Some(json!({ "payload": { "text": "pwned" } }))),
        ("POST", format!("/v1/units/{acme}/archive"), None),
        ("DELETE", format!("/v1/units/{acme}"), None),
        ("GET", format!("/v1/units/{acme}/relations"), None),
    ] {
        let reply = as_user(&server, GLOBEX, method, &path, body.as_ref());
        assert_eq!((reply.status, reply.json()["error"]["code"].as_str()), (404, Some("not_found")), "{method} {path}");
    }
    assert_eq!(listed(&server, GLOBEX), [globex.as_str()]);
    assert_eq!(listed(&server, ACME), [acme.as_str()]);
    let search = as_user(&server, GLOBEX, "POST", "/v1/search", Some(&json!({ "text": "secret" })));
    let search = search.json();
    let hits: Vec<&str> = search["hits"].as_array().unwrap().iter().map(|h| h["id"].as_str().unwrap()).collect();
    assert_eq!(hits, [globex.as_str()]);
    let query = as_user(&server, GLOBEX, "POST", "/v1/query", Some(&json!({ "query": "FIND note AS n" })));
    assert_eq!(query.json()["rows"].as_array().unwrap().len(), 1);
    // A relation to another tenant's unit has no unit at the other end.
    let relate = as_user(
        &server,
        GLOBEX,
        "POST",
        "/v1/relations",
        Some(&json!({ "from": globex, "to": acme, "type": "steals" })),
    );
    assert_eq!(relate.status, 404);

    // acme's unit is untouched.
    let read = as_user(&server, ACME, "GET", &format!("/v1/units/{acme}"), None).json();
    assert_eq!((read["revision"].as_u64(), read["lifecycle"].as_str()), (Some(1), Some("active")));

    // One directory per tenant, under the data directory, named after the tenant.
    let mut dirs: Vec<String> =
        std::fs::read_dir(tenants.root()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    dirs.sort();
    assert_eq!(dirs, ["acme", "globex"]);
}

#[test]
fn a_tenant_keeps_its_data_across_a_restart() {
    let tenants = Tenants::new();
    let server = tenants.start(&[]);
    let id = create_note(&server, GLOBEX, "durable");
    assert!(server.stop().success());
    let again = tenants.start(&[]);
    assert_eq!(as_user(&again, GLOBEX, "GET", &format!("/v1/units/{id}"), None).status, 200);
    assert_eq!(as_user(&again, ACME, "GET", &format!("/v1/units/{id}"), None).status, 404);
}

#[test]
fn a_bad_tokens_file_or_a_missing_half_stops_the_server_before_it_listens() {
    let tenants = Tenants::new();
    let bad = tenants.dir.0.join("bad-tokens");
    let run = |args: &[&str]| {
        let out =
            Command::new(env!("CARGO_BIN_EXE_kerno-server")).env_remove("KERNO_SERVER_DB").args(args).output().unwrap();
        assert!(!out.status.success(), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?} listened");
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let root = tenants.root();
    let root = root.to_str().unwrap();
    std::fs::write(&bad, format!("{} ../escape\n", hash_hex(ACME))).unwrap();
    let stderr = run(&["--data-dir", root, "--tokens", bad.to_str().unwrap()]);
    assert!(stderr.contains("line 1: tenant \"../escape\""), "{stderr}");
    // Nothing was created outside, or inside, the data directory.
    assert!(!tenants.dir.0.join("escape").exists());

    let missing = tenants.dir.0.join("no-such-file");
    let stderr = run(&["--data-dir", root, "--tokens", missing.to_str().unwrap()]);
    assert!(stderr.contains("cannot read the tokens file"), "{stderr}");

    let stderr = run(&["--data-dir", root]);
    assert!(stderr.contains("--data-dir needs --tokens") && stderr.contains("usage:"), "{stderr}");
}

#[test]
fn hash_token_prints_the_line_for_the_tokens_file() {
    let hash = |stdin: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kerno-server"))
            .arg("hash-token")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    let out = hash(&format!("{ACME}\n"));
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{}\n", hash_hex(ACME)));
    for weak in ["short", "", &format!("{ACME} {ACME}")] {
        let out = hash(weak);
        assert!(!out.status.success(), "{weak:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("a token is at least 32 characters"), "{weak:?}");
    }
    // Exactly 32 characters is enough.
    assert!(hash(&"x".repeat(32)).status.success());
    // `hash-token` takes no argument: with one it is not the subcommand.
    let out = Command::new(env!("CARGO_BIN_EXE_kerno-server")).args(["hash-token", ACME]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown argument \"hash-token\""));
}

// ------------------------------------------------------------ health, metrics

#[test]
fn the_probes_answer_without_a_token() {
    let tenants = Tenants::new();
    let server = tenants.start(&[]);
    let live = server.get("/livez");
    assert_eq!((live.status, live.json()), (200, json!({ "status": "alive" })));
    let ready = server.get("/readyz");
    assert_eq!((ready.status, ready.json()), (200, json!({ "status": "ready" })));
    for path in ["/livez", "/readyz", "/metrics"] {
        let reply = server.post(path, &json!({}));
        assert_eq!((reply.status, reply.header("allow")), (405, Some("GET")), "{path}");
    }
    // Only the exact paths: anything else outside /v1 is a 404, not a probe.
    assert_eq!(server.get("/livez/x").status, 404);
    assert_eq!(server.get("/").status, 404);
}

#[test]
fn readiness_fails_while_a_tenant_directory_is_gone_and_liveness_does_not() {
    let tenants = Tenants::new();
    let server = tenants.start(&[]);
    assert_eq!(server.get("/readyz").status, 200);
    // globex is the second database the server holds: every one is checked, not only the first.
    let (dir, moved) = (tenants.root().join("globex"), tenants.root().join("globex-moved"));
    std::fs::rename(&dir, &moved).unwrap();
    let ready = server.get("/readyz");
    assert_eq!(ready.status, 503);
    assert_eq!(ready.json(), json!({ "error": { "code": "unavailable", "message": "a database directory is gone" } }));
    assert_eq!(server.get("/livez").status, 200);
    std::fs::rename(&moved, &dir).unwrap();
    assert_eq!(server.get("/readyz").status, 200);
}

#[test]
fn readiness_of_one_database_checks_its_directory() {
    let dir = TempDir::new();
    let server = Server::start(&dir.db());
    assert_eq!(server.get("/readyz").status, 200);
    let moved = dir.0.join("moved");
    std::fs::rename(dir.db(), &moved).unwrap();
    assert_eq!(server.get("/readyz").status, 503);
    std::fs::rename(&moved, dir.db()).unwrap();
}

#[test]
fn metrics_count_answers_and_refusals_and_name_no_tenant() {
    let tenants = Tenants::new();
    let server = tenants.start(&[]);
    create_note(&server, ACME, "x");
    as_user(&server, GLOBEX, "GET", "/v1/units?type=note", None);
    server.get("/v1/health");
    server.call("GET", "/v1/health", &[("Authorization", "Bearer nope")], None);

    let reply = server.get("/metrics");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("content-type"), Some("text/plain; version=0.0.4"));
    let text = String::from_utf8(reply.body).unwrap();
    assert_eq!(metric(&text, "kerno_http_responses_total{status=\"201\"}"), Some(1), "{text}");
    assert_eq!(metric(&text, "kerno_http_responses_total{status=\"200\"}"), Some(1), "{text}");
    assert_eq!(metric(&text, "kerno_http_responses_total{status=\"401\"}"), Some(2), "{text}");
    assert_eq!(metric(&text, "kerno_auth_failures_total"), Some(2), "{text}");
    assert_eq!(metric(&text, "kerno_request_duration_seconds_count{transport=\"http\"}"), Some(4), "{text}");
    assert_eq!(metric(&text, "kerno_databases_open"), Some(2), "{text}");
    for secret in ["acme", "globex", ACME, GLOBEX] {
        assert!(!text.contains(secret), "{secret} in\n{text}");
    }
    // The scrape itself is counted on the next one.
    let again = String::from_utf8(server.get("/metrics").body).unwrap();
    assert_eq!(metric(&again, "kerno_http_responses_total{status=\"200\"}"), Some(2), "{again}");
}

// ------------------------------------------------------------------- logs

#[test]
fn logs_are_json_lines_that_name_the_tenant_and_never_a_token() {
    let tenants = Tenants::new();
    let server = tenants.start(&[]);
    create_note(&server, ACME, "x");
    server.call("GET", "/v1/health", &[("Authorization", &bearer(GLOBEX.replace('1', "2").as_str()))], None);
    server.get("/v1/health");
    assert!(server.stop().success());

    let lines = tenants.log_lines();
    let text = std::fs::read_to_string(tenants.log()).unwrap();
    for token in [ACME, ACME_TOO, GLOBEX, &GLOBEX.replace('1', "2")] {
        assert!(!text.contains(token), "a token was logged:\n{text}");
    }
    let message = |l: &Value| l["fields"]["message"].as_str().unwrap_or_default().to_owned();
    let created = lines
        .iter()
        .find(|l| message(l) == "request" && l["fields"]["status"] == 201)
        .unwrap_or_else(|| panic!("no request line:\n{text}"));
    assert_eq!(created["level"], "INFO");
    assert_eq!(created["fields"]["transport"], "http");
    assert_eq!(created["fields"]["method"], "POST");
    assert_eq!(created["fields"]["path"], "/v1/units");
    assert_eq!(created["fields"]["tenant"], "acme");
    assert!(created["fields"]["duration_us"].is_u64());
    assert!(created["timestamp"].is_string());
    // A refused request has no tenant; the warning says why, never with the token.
    let refused: Vec<&Value> = lines.iter().filter(|l| message(l) == "request refused").collect();
    let reasons: Vec<&str> = refused.iter().map(|l| l["fields"]["reason"].as_str().unwrap()).collect();
    assert_eq!(reasons, ["unknown token", "no bearer token"]);
    assert!(refused.iter().all(|l| l["level"] == "WARN"));
    let unauthorized = lines.iter().find(|l| message(l) == "request" && l["fields"]["status"] == 401).unwrap();
    assert!(unauthorized["fields"].get("tenant").is_none(), "{unauthorized}");
    assert!(lines.iter().any(|l| message(l) == "listening" && l["fields"]["authentication"] == true));
    assert!(lines.iter().any(|l| message(l) == "stopping"));
}

#[test]
fn a_stop_sent_the_moment_the_server_listens_is_graceful() {
    // The signal handlers are installed before the ports are printed; before that fix, about one
    // stop in two sent right after the line killed the process instead. The shell's built-in
    // `kill` and a busy wait are fast enough to hit that window; spawning `kill` is not.
    let dir = TempDir::new();
    // Ten times: start, and ask to stop the moment the address is printed. Every exit must be
    // the graceful one (status 0). Written with the harness, not `sh`, so it runs on Windows
    // too (issue #4).
    for _ in 0..10 {
        let db = dir.0.join("db");
        let server = Server::launch(&["--db", db.to_str().unwrap()], None);
        assert!(server.stop().success(), "the server did not stop gracefully");
    }
}

#[test]
fn serving_beyond_loopback_without_protection_is_warned_about() {
    let warnings = |args: &[String], log: &std::path::Path| {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let server = Server::launch(&args, Some(log));
        assert!(server.stop().success());
        let text = std::fs::read_to_string(log).unwrap();
        text.lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|l| l["level"] == "WARN")
            .map(|l| l["fields"]["message"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let tenants = Tenants::new();
    let db = tenants.dir.0.join("db").to_str().unwrap().to_owned();
    let log = tenants.log();
    let any = || vec!["--listen".to_owned(), "0.0.0.0:0".to_owned()];

    let open = warnings(&[vec!["--db".into(), db.clone()], any()].concat(), &log);
    assert_eq!(open.len(), 1);
    assert!(open[0].contains("without authentication"), "{open:?}");
    let clear = warnings(&tenants.args(&any()), &log);
    assert_eq!(clear.len(), 1);
    assert!(clear[0].contains("without TLS"), "{clear:?}");
    // A gRPC port beyond loopback counts too.
    let grpc = ["--grpc-listen".to_owned(), "0.0.0.0:0".to_owned()];
    assert_eq!(warnings(&[vec!["--db".into(), db.clone()], grpc.to_vec()].concat(), &log).len(), 1);

    // Protected, or on loopback: no warning.
    assert!(warnings(&tenants.args(&[any(), tls_args()].concat()), &log).is_empty());
    assert!(warnings(&["--db".into(), db], &log).is_empty());
}

// -------------------------------------------------------------------- TLS

#[test]
fn https_serves_the_api_and_a_plaintext_client_gets_nothing() {
    let tenants = Tenants::new();
    let server = tenants.start(&tls_args());
    assert!(server.https);
    assert_eq!(server.call("GET", "/livez", &[], None).status, 200);
    let id = create_note(&server, ACME, "over TLS");
    assert_eq!(as_user(&server, ACME, "GET", &format!("/v1/units/{id}"), None).json()["payload"]["text"], "over TLS");
    assert_unauthenticated(&server.call("GET", "/v1/health", &[], None), "TLS without a token");

    // Plain HTTP on the TLS port: the handshake fails and no HTTP answer comes back.
    use std::io::Read;
    let mut tcp = std::net::TcpStream::connect(server.http).unwrap();
    tcp.write_all(b"GET /livez HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    let _ = tcp.read_to_end(&mut raw);
    assert!(!raw.starts_with(b"HTTP/"), "{:?}", String::from_utf8_lossy(&raw));
    // The server is still up for the next client.
    assert_eq!(server.call("GET", "/livez", &[], None).status, 200);
}

#[test]
fn a_client_that_never_finishes_its_tls_handshake_is_cut_off() {
    use std::io::Read;
    let dir = TempDir::new();
    let args = [vec!["--db".to_owned(), dir.db().to_str().unwrap().to_owned()], tls_args()].concat();
    let server = Server::launch(&args.iter().map(String::as_str).collect::<Vec<_>>(), None);
    let mut silent = std::net::TcpStream::connect(server.http).unwrap();
    // The server allows 10 seconds; a read that outlives 20 means it never hung up.
    silent.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
    let started = std::time::Instant::now();
    let mut byte = [0; 1];
    let read = silent.read(&mut byte);
    assert!(
        matches!(read, Ok(0)) || read.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset),
        "{read:?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(15), "closed after {:?}", started.elapsed());
}

#[test]
fn a_certificate_or_key_that_does_not_load_stops_the_server() {
    let dir = TempDir::new();
    let not_pem = dir.0.join("not.pem");
    std::fs::write(&not_pem, "not a certificate").unwrap();
    let cert = tls_file("server.pem");
    let key = tls_file("server.key");
    let db = dir.db();
    for (c, k, says) in [
        (&not_pem, &key, "no certificate in the file"),
        (&cert, &not_pem, "--tls-key"),
        (&dir.0.join("missing.pem"), &key, "cannot read"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_kerno-server"))
            .env_remove("KERNO_SERVER_DB")
            .arg("--db")
            .arg(&db)
            .args(["--listen", "127.0.0.1:0", "--tls-cert"])
            .arg(c)
            .arg("--tls-key")
            .arg(k)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(out.stdout.is_empty(), "it listened");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(says), "{says}: {stderr}");
    }
}

// ------------------------------------------------------------------- gRPC

type Client = KernoClient<Channel>;

async fn grpc_client(server: &Server, tls: bool) -> Client {
    let addr = server.grpc.unwrap();
    if !tls {
        return KernoClient::connect(format!("http://{addr}")).await.unwrap();
    }
    let ca = Certificate::from_pem(std::fs::read(tls_file("ca.pem")).unwrap());
    let channel = Channel::from_shared(format!("https://{addr}"))
        .unwrap()
        .tls_config(ClientTlsConfig::new().ca_certificate(ca).domain_name("localhost"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    KernoClient::new(channel)
}

fn signed<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(AUTHORIZATION_KEY, bearer(token).parse().unwrap());
    request
}

fn grpc_tenants(tenants: &Tenants, extra: &[String]) -> Server {
    tenants.start(&[vec!["--grpc-listen".to_owned(), "127.0.0.1:0".to_owned()], extra.to_vec()].concat())
}

#[tokio::test]
async fn grpc_refuses_a_call_without_a_valid_token() {
    let tenants = Tenants::new();
    let server = grpc_tenants(&tenants, &[]);
    let mut c = grpc_client(&server, false).await;
    let unit = || pb::UnitRequest { id: "00".repeat(16), expected_revision: None };
    for request in [Request::new(unit()), signed(unit(), "nope"), signed(unit(), &hash_hex(ACME))] {
        let status = c.get_unit(request).await.unwrap_err();
        assert_eq!(status.code(), Code::Unauthenticated);
        assert_eq!(status.metadata().get(ERROR_CODE_KEY).unwrap(), "unauthenticated");
    }
    // Checked before the message is read: a malformed payload is still a 401, not a 400.
    let create = pb::CreateUnitRequest { unit_type: "note".into(), payload: None };
    assert_eq!(c.create_unit(Request::new(create.clone())).await.unwrap_err().code(), Code::Unauthenticated);
    assert_eq!(c.create_unit(signed(create, ACME)).await.unwrap_err().code(), Code::InvalidArgument);
    assert_eq!(c.health(Request::new(pb::Empty {})).await.unwrap_err().code(), Code::Unauthenticated);
    assert_eq!(c.health(signed(pb::Empty {}, GLOBEX)).await.unwrap().into_inner().status, "ok");

    let text = String::from_utf8(server.get("/metrics").body).unwrap();
    assert_eq!(metric(&text, "kerno_grpc_calls_total{code=\"unauthenticated\"}"), Some(5), "{text}");
    assert_eq!(metric(&text, "kerno_grpc_calls_total{code=\"invalid_request\"}"), Some(1), "{text}");
    assert_eq!(metric(&text, "kerno_grpc_calls_total{code=\"ok\"}"), Some(1), "{text}");
    assert_eq!(metric(&text, "kerno_auth_failures_total"), Some(5), "{text}");
    assert_eq!(metric(&text, "kerno_request_duration_seconds_count{transport=\"grpc\"}"), Some(7), "{text}");
}

#[tokio::test]
async fn grpc_reaches_the_tenant_of_its_token_and_the_same_one_as_http() {
    let tenants = Tenants::new();
    let server = grpc_tenants(&tenants, &[]);
    let mut c = grpc_client(&server, false).await;
    let text = |t: &str| Some(pb::Payload { kind: Some(pb::payload::Kind::Text(t.into())) });
    let create = pb::CreateUnitRequest { unit_type: "note".into(), payload: text("acme only") };
    let id = c.create_unit(signed(create, ACME)).await.unwrap().into_inner().id;

    let get = || pb::UnitRequest { id: id.clone(), expected_revision: None };
    assert_eq!(c.get_unit(signed(get(), ACME_TOO)).await.unwrap().into_inner().id, id);
    assert_eq!(c.get_unit(signed(get(), GLOBEX)).await.unwrap_err().code(), Code::NotFound);
    assert_eq!(c.delete_unit(signed(get(), GLOBEX)).await.unwrap_err().code(), Code::NotFound);
    let list = || pb::ListUnitsRequest { unit_type: Some("note".into()), limit: None, after: None };
    assert!(c.list_units(signed(list(), GLOBEX)).await.unwrap().into_inner().units.is_empty());
    assert_eq!(c.list_units(signed(list(), ACME)).await.unwrap().into_inner().units.len(), 1);
    // HTTP with the same tenant's token sees what gRPC wrote; the other tenant does not.
    assert_eq!(as_user(&server, ACME, "GET", &format!("/v1/units/{id}"), None).status, 200);
    assert_eq!(as_user(&server, GLOBEX, "GET", &format!("/v1/units/{id}"), None).status, 404);
}

#[tokio::test]
async fn grpc_over_tls_takes_a_trusting_client_and_refuses_a_plaintext_one() {
    let tenants = Tenants::new();
    let server = grpc_tenants(&tenants, &tls_args());
    let mut c = grpc_client(&server, true).await;
    assert_eq!(c.health(signed(pb::Empty {}, ACME)).await.unwrap().into_inner().status, "ok");
    assert_eq!(c.health(Request::new(pb::Empty {})).await.unwrap_err().code(), Code::Unauthenticated);

    let plain = format!("http://{}", server.grpc.unwrap());
    let refused = match KernoClient::connect(plain).await {
        Err(_) => true,
        Ok(mut c) => c.health(signed(pb::Empty {}, ACME)).await.is_err(),
    };
    assert!(refused, "a plaintext gRPC client got an answer from a TLS port");
}
