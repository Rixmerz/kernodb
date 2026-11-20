//! The S3 client against a mock S3 server in this process: it checks the SigV4 signature of every
//! request with its own implementation of the algorithm, so what is signed is what is sent.

// The no-panic rule is for the library; a test panics to fail.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kerno::Db;
use kerno_backup::s3::{S3Config, S3Store};
use kerno_backup::{Keeper, ObjectStore, Prefixed, Target, restore};
use ring::{digest, hmac};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

const ACCESS: &str = "AKIATESTACCESSKEY";
const SECRET: &str = "test-secret-key-that-must-never-be-echoed";
const PAGE: usize = 2;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}

fn mac(key: &[u8], data: &str) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data.as_bytes()).as_ref().to_vec()
}

struct Request {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// Whether the signature of `r` is the one `secret` makes, computed independently of the client.
fn signed_by(r: &Request, secret: &str) -> bool {
    let Some(auth) = r.headers.get("authorization") else { return false };
    let field = |name: &str| auth.split(&format!("{name}=")).nth(1).and_then(|s| s.split([',', ' ']).next());
    let (Some(signed), Some(got)) = (field("SignedHeaders"), field("Signature")) else { return false };
    let (path, query) = r.target.split_once('?').unwrap_or((&r.target, ""));
    let headers: String = signed.split(';').map(|h| format!("{h}:{}\n", r.headers[h])).collect();
    let canonical =
        format!("{}\n{path}\n{query}\n{headers}\n{signed}\n{}", r.method, r.headers["x-amz-content-sha256"]);
    let date = &r.headers["x-amz-date"];
    let scope = format!("{}/us-east-1/s3/aws4_request", &date[..8]);
    let to_sign = format!("AWS4-HMAC-SHA256\n{date}\n{scope}\n{}", sha(canonical.as_bytes()));
    let mut key = mac(format!("AWS4{secret}").as_bytes(), &date[..8]);
    for part in ["us-east-1", "s3", "aws4_request"] {
        key = mac(&key, part);
    }
    auth.contains(&format!("Credential={ACCESS}/{scope}")) && hex(&mac(&key, &to_sign)) == got
}

fn read_request(stream: &mut impl Read) -> Option<Request> {
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    while !raw.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        raw.push(byte[0]);
    }
    let head = String::from_utf8(raw).ok()?;
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: BTreeMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let mut body = vec![0u8; headers.get("content-length")?.parse().ok()?];
    stream.read_exact(&mut body).ok()?;
    Some(Request { method, target, headers, body })
}

fn xml_error(status: u16, code: &str) -> (u16, Vec<u8>, bool) {
    (status, format!("<Error><Code>{code}</Code><Message>mock says {code}</Message></Error>").into_bytes(), false)
}

fn param(query: &str, name: &str) -> String {
    let raw = query.split('&').find_map(|p| p.strip_prefix(&format!("{name}="))).unwrap_or("");
    let mut out = Vec::new();
    let mut bytes = raw.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next().unwrap();
            let lo = bytes.next().unwrap();
            out.push(u8::from_str_radix(&format!("{}{}", hi as char, lo as char), 16).unwrap());
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).unwrap()
}

/// `(status, body, chunked)` for one request.
fn answer(r: &Request, objects: &Mutex<BTreeMap<String, Vec<u8>>>) -> (u16, Vec<u8>, bool) {
    if !signed_by(r, SECRET) {
        return xml_error(403, "SignatureDoesNotMatch");
    }
    if sha(&r.body) != r.headers["x-amz-content-sha256"] {
        return xml_error(400, "XAmzContentSHA256Mismatch");
    }
    let (path, query) = r.target.split_once('?').unwrap_or((&r.target, ""));
    let mut objects = objects.lock().unwrap();
    let Some(key) = path.strip_prefix("/bucket/") else {
        // The bucket itself: a listing, a page of PAGE keys at a time, answered chunked.
        assert_eq!((r.method.as_str(), path, param(query, "list-type").as_str()), ("GET", "/bucket", "2"));
        let (prefix, after) = (param(query, "prefix"), param(query, "continuation-token"));
        let matching: Vec<&String> = objects.keys().filter(|k| k.starts_with(&prefix) && **k > after).collect();
        let page = &matching[..matching.len().min(PAGE)];
        let mut xml = String::from("<ListBucketResult>");
        for k in page {
            xml.push_str(&format!("<Contents><Key>{k}</Key></Contents>"));
        }
        if matching.len() > PAGE {
            xml.push_str(&format!(
                "<IsTruncated>true</IsTruncated><NextContinuationToken>{}</NextContinuationToken>",
                page[PAGE - 1]
            ));
        } else {
            xml.push_str("<IsTruncated>false</IsTruncated>");
        }
        xml.push_str("</ListBucketResult>");
        return (200, xml.into_bytes(), true);
    };
    match r.method.as_str() {
        "PUT" => {
            objects.insert(key.to_owned(), r.body.clone());
            (200, Vec::new(), false)
        }
        "GET" => match objects.get(key) {
            Some(v) => (200, v.clone(), false),
            None => xml_error(404, "NoSuchKey"),
        },
        "DELETE" => {
            objects.remove(key);
            (204, Vec::new(), false)
        }
        other => panic!("unexpected {other}"),
    }
}

fn respond(stream: &mut impl Write, (status, body, chunked): (u16, Vec<u8>, bool)) {
    let mut out = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n").into_bytes();
    if chunked {
        out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
        for part in body.chunks(7) {
            out.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
            out.extend_from_slice(part);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"0\r\n\r\n");
    } else {
        out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        out.extend_from_slice(&body);
    }
    let _ = stream.write_all(&out);
}

fn tls_file(name: &str) -> PathBuf {
    // The certificates of the kerno-server tests: a CA, and a certificate for 127.0.0.1 it signed.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../kerno-server/tests/tls").join(name)
}

struct Mock {
    addr: SocketAddr,
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl Mock {
    fn start(tls: bool) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let objects = Arc::new(Mutex::new(BTreeMap::new()));
        let config = tls.then(|| {
            let certs: Vec<_> =
                CertificateDer::pem_file_iter(tls_file("server.pem")).unwrap().map(Result::unwrap).collect();
            let key = PrivateKeyDer::from_pem_file(tls_file("server.key")).unwrap();
            Arc::new(
                rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .with_no_client_auth()
                    .with_single_cert(certs, key)
                    .unwrap(),
            )
        });
        let shared = Arc::clone(&objects);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (Ok(stream), objects) = (stream, Arc::clone(&shared)) else { continue };
                let config = config.clone();
                std::thread::spawn(move || serve(stream, config, &objects));
            }
        });
        Mock { addr, objects }
    }

    fn store(&self, tls: bool, secret: &str, ca: bool) -> S3Store {
        S3Store::new(
            "bucket",
            S3Config {
                endpoint: format!("{}://{}", if tls { "https" } else { "http" }, self.addr),
                region: "us-east-1".into(),
                access_key: ACCESS.into(),
                secret_key: secret.into(),
                session_token: None,
                ca_file: (tls && ca).then(|| tls_file("ca.pem")),
            },
        )
        .unwrap()
    }
}

fn serve(stream: TcpStream, tls: Option<Arc<rustls::ServerConfig>>, objects: &Mutex<BTreeMap<String, Vec<u8>>>) {
    match tls {
        Some(config) => {
            let conn = rustls::ServerConnection::new(config).unwrap();
            let mut stream = rustls::StreamOwned::new(conn, stream);
            if let Some(r) = read_request(&mut stream) {
                respond(&mut stream, answer(&r, objects));
                stream.conn.send_close_notify();
                let _ = stream.flush();
            }
        }
        None => {
            let mut stream = stream;
            if let Some(r) = read_request(&mut stream) {
                respond(&mut stream, answer(&r, objects));
            }
        }
    }
}

fn exercise(store: &dyn ObjectStore, mock: &Mock) {
    assert_eq!(store.get("wal/missing").unwrap(), None);
    for (i, key) in ["base/1/MANIFEST", "base/1/part-000000", "wal/a-1", "wal/a-2", "wal/a-3"].iter().enumerate() {
        store.put(key, format!("body {i}").as_bytes()).unwrap();
    }
    store.put("wal/a-2", b"replaced").unwrap();
    assert_eq!(store.get("wal/a-2").unwrap().as_deref(), Some(&b"replaced"[..]));
    // Five keys, two to a page.
    assert_eq!(store.list("").unwrap().len(), 5);
    assert_eq!(store.list("wal/").unwrap(), ["wal/a-1", "wal/a-2", "wal/a-3"]);
    assert_eq!(store.list("nothing").unwrap(), Vec::<String>::new());
    store.delete("wal/a-1").unwrap();
    store.delete("wal/a-1").unwrap();
    assert_eq!(store.list("wal/").unwrap(), ["wal/a-2", "wal/a-3"]);
    assert_eq!(mock.objects.lock().unwrap().len(), 4);
}

#[test]
fn objects_are_put_read_listed_page_by_page_and_deleted_over_http() {
    let mock = Mock::start(false);
    exercise(&mock.store(false, SECRET, false), &mock);
}

#[test]
fn the_same_over_tls_with_the_ca_of_the_store() {
    let mock = Mock::start(true);
    exercise(&mock.store(true, SECRET, true), &mock);
    // Without that CA the certificate is not trusted, and nothing is sent.
    let err = mock.store(true, SECRET, false).put("k", b"v").unwrap_err().to_string();
    assert!(err.contains("the store at 127.0.0.1"), "{err}");
    assert!(!mock.objects.lock().unwrap().contains_key("k"));
}

#[test]
fn a_refusal_names_the_code_and_never_the_secret() {
    let mock = Mock::start(false);
    let wrong = mock.store(false, "another-secret", false);
    for err in [
        wrong.put("k", b"v").unwrap_err().to_string(),
        wrong.get("k").unwrap_err().to_string(),
        wrong.list("").unwrap_err().to_string(),
        wrong.delete("k").unwrap_err().to_string(),
    ] {
        assert!(err.contains("403 SignatureDoesNotMatch mock says SignatureDoesNotMatch"), "{err}");
        assert!(!err.contains(SECRET) && !err.contains("another-secret") && !err.contains(ACCESS), "{err}");
    }
    assert!(mock.objects.lock().unwrap().is_empty());
    // A closed port is an error that names the store, not a panic.
    let gone = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        Mock { addr, objects: Arc::default() }
    };
    let err = gone.store(false, SECRET, false).get("k").unwrap_err().to_string();
    assert!(err.starts_with("GET on the store at 127.0.0.1:"), "{err}");
}

#[test]
fn a_database_is_backed_up_to_s3_and_restored_from_it() {
    let dir = std::env::temp_dir().join(format!("kb-s3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mock = Mock::start(false);
    let s3: Arc<dyn ObjectStore> = Arc::new(Prefixed::new(Arc::new(mock.store(false, SECRET, false)), "tenant-a"));

    let db = Db::open(dir.join("db")).unwrap();
    db.store("note", "before the base").unwrap();
    let mut keeper = Keeper::new(&dir.join("db"), Arc::clone(&s3));
    assert!(keeper.tick(db.unit_db(), 1).unwrap().base.is_some());
    db.store("note", "after the base").unwrap();
    assert_eq!(keeper.tick(db.unit_db(), 2).unwrap().shipped.commits, 1);
    let want = kerno::units::export::export(db.unit_db()).unwrap();
    db.close().unwrap();

    assert!(mock.objects.lock().unwrap().keys().all(|k| k.starts_with("tenant-a/")));
    let out = dir.join("restored");
    restore(&*s3, &out, Target::Latest).unwrap();
    let copy = Db::open(&out).unwrap();
    assert_eq!(kerno::units::export::export(copy.unit_db()).unwrap(), want);
    copy.close().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
