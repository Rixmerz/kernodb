//! Starts the real `kerno-server` binary on free ports and talks to it over a socket.

// The no-panic rule is for the server; a test panics to fail.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value;

/// A fresh directory that is removed when the test ends, pass or fail. It lives under the system
/// temp directory: a test runs with the crate as its working directory, and a database written
/// there would end up in `git add -A`.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kerno-server-e2e-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    /// Two levels that do not exist yet: the server must create both.
    pub fn db(&self) -> PathBuf {
        self.0.join("data").join("db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", String::from_utf8_lossy(&self.body)))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub struct Server {
    child: Child,
    // Kept open so the server never writes into a closed pipe.
    _stdout: BufReader<ChildStdout>,
    pub http: SocketAddr,
    pub grpc: Option<SocketAddr>,
    /// The HTTP side speaks TLS: it printed `listening on https://`.
    pub https: bool,
}

/// The test certificate authority, and a certificate and key it signed for `localhost` and
/// `127.0.0.1`, valid until 2126. Made once with `openssl`, for these tests only.
pub fn tls_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tls").join(name)
}

/// The arguments that turn TLS on with the test certificate.
pub fn tls_args() -> Vec<String> {
    let path = |n: &str| tls_file(n).to_str().unwrap().to_owned();
    vec!["--tls-cert".into(), path("server.pem"), "--tls-key".into(), path("server.key")]
}

impl Server {
    /// The server on `db`, HTTP only.
    pub fn start(db: &Path) -> Server {
        Self::start_with(db, &[])
    }

    /// The server on `db` with `extra` arguments after `--db <db> --listen 127.0.0.1:0`.
    pub fn start_with(db: &Path, extra: &[&str]) -> Server {
        let mut args = vec!["--db", db.to_str().unwrap()];
        args.extend_from_slice(extra);
        Self::launch(&args, None)
    }

    /// The server with `--listen 127.0.0.1:0` then `args`; its stderr (the JSON logs) goes to
    /// `log` when given.
    pub fn launch(args: &[&str], log: Option<&Path>) -> Server {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_kerno-server"));
        cmd.env_remove("KERNO_SERVER_DB").args(["--listen", "127.0.0.1:0"]).args(args);
        let stderr = match log {
            Some(path) => Stdio::from(std::fs::File::create(path).unwrap()),
            None => Stdio::inherit(),
        };
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(stderr).spawn().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let wants_grpc = args.contains(&"--grpc-listen");
        let (mut http, mut grpc, mut https) = (None, None, false);
        while http.is_none() || (wants_grpc && grpc.is_none()) {
            let mut line = String::new();
            assert!(stdout.read_line(&mut line).unwrap() > 0, "the server exited before it listened");
            if let Some(addr) = line.trim().strip_prefix("listening on http://") {
                http = Some(addr.parse().unwrap());
            } else if let Some(addr) = line.trim().strip_prefix("listening on https://") {
                (http, https) = (Some(addr.parse().unwrap()), true);
            } else if let Some(addr) = line.trim().strip_prefix("grpc listening on ") {
                grpc = Some(addr.parse().unwrap());
            }
        }
        Server { child, _stdout: stdout, http: http.unwrap(), grpc, https }
    }

    /// One request with extra `headers` (`Authorization`, say), over TLS when the server has it.
    pub fn call(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Reply {
        let body = body.map(|b| b.to_string().into_bytes());
        let raw = request_bytes(self.http, method, path, headers, body.as_deref());
        let mut tcp = TcpStream::connect(self.http).unwrap();
        let mut answer = Vec::new();
        if self.https {
            let mut tls = rustls::StreamOwned::new(tls_client(), tcp);
            tls.write_all(&raw).unwrap();
            // A peer that closes without close_notify still sent the whole answer before.
            match tls.read_to_end(&mut answer) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
                Err(e) => panic!("{e}"),
            }
        } else {
            tcp.write_all(&raw).unwrap();
            tcp.read_to_end(&mut answer).unwrap();
        }
        parse_reply(&answer)
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Reply {
        raw_request(self.http, method, path, body.map(|b| b.to_string().into_bytes()).as_deref())
    }

    pub fn get(&self, path: &str) -> Reply {
        self.request("GET", path, None)
    }

    pub fn post(&self, path: &str, body: &Value) -> Reply {
        self.request("POST", path, Some(body))
    }

    pub fn put(&self, path: &str, body: &Value) -> Reply {
        self.request("PUT", path, Some(body))
    }

    pub fn delete(&self, path: &str) -> Reply {
        self.request("DELETE", path, None)
    }

    /// Asks the server to stop as a service manager does (SIGTERM) and waits for it to exit.
    pub fn stop(mut self) -> ExitStatus {
        let status = Command::new("kill").arg("-TERM").arg(self.child.id().to_string()).status().unwrap();
        assert!(status.success(), "kill -TERM failed");
        self.child.wait().unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A rustls client that trusts the test certificate authority, for `localhost`.
pub fn tls_client() -> rustls::ClientConnection {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from_pem_file(tls_file("ca.pem")).unwrap()).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    rustls::ClientConnection::new(Arc::new(config), "localhost".try_into().unwrap()).unwrap()
}

fn request_bytes(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Vec<u8> {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = body {
        head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut raw = head.into_bytes();
    raw.extend_from_slice(body.unwrap_or_default());
    raw
}

/// One HTTP/1.1 request on a fresh connection, read to the end of the answer.
pub fn raw_request(addr: SocketAddr, method: &str, path: &str, body: Option<&[u8]>) -> Reply {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(&request_bytes(addr, method, path, &[], body)).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    parse_reply(&raw)
}

pub fn parse_reply(raw: &[u8]) -> Reply {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("no end of headers") + 4;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    Reply { status, headers, body: raw[split..].to_vec() }
}
