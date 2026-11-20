//! S3 and S3-compatible stores (R2, MinIO): path-style requests signed with AWS Signature V4, over
//! one HTTP/1.1 connection per request, plain or TLS.
//!
//! No SDK: a backup needs four calls (PUT, GET, DELETE and list) and a signature that `ring`
//! computes. The credentials come from the environment and never reach an error message or a log.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ring::{digest, hmac};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

use crate::base::hex;
use crate::error::{Error, Result};
use crate::store::{ObjectStore, check_key};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Everything an [`S3Store`] needs. Not `Debug`: it holds the secret.
pub struct S3Config {
    /// `https://host[:port]` or `http://host[:port]`; the host of the store.
    pub endpoint: String,
    /// The signing region (`auto` for R2, `us-east-1` for most others).
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
    /// A PEM file of certificate authorities to trust besides the system's.
    pub ca_file: Option<PathBuf>,
}

pub struct S3Store {
    config: S3Config,
    bucket: String,
    scheme_tls: bool,
    host: String,
    port: u16,
    tls: Option<Arc<rustls::ClientConfig>>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The configuration `lookup` (the environment, or a test's map) names.
fn config_from(lookup: &dyn Fn(&str) -> Option<String>) -> Result<S3Config> {
    let need = |name: &str| lookup(name).ok_or_else(|| Error::Store(format!("{name} is not set")));
    let region = lookup("KERNO_S3_REGION").unwrap_or_else(|| "us-east-1".into());
    Ok(S3Config {
        endpoint: lookup("KERNO_S3_ENDPOINT").unwrap_or_else(|| format!("https://s3.{region}.amazonaws.com")),
        region,
        access_key: need("AWS_ACCESS_KEY_ID")?,
        secret_key: need("AWS_SECRET_ACCESS_KEY")?,
        session_token: lookup("AWS_SESSION_TOKEN"),
        ca_file: lookup("KERNO_S3_CA_FILE").map(PathBuf::from),
    })
}

/// Seconds since the Unix epoch as `(YYYYMMDDTHHMMSSZ, YYYYMMDD)`.
fn amz_date(secs: u64) -> (String, String) {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let date = format!("{year:04}{month:02}{day:02}");
    (format!("{date}T{:02}{:02}{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60), date)
}

fn sha256_hex(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data).as_ref().to_vec()
}

/// SigV4 percent-encoding: everything but `A-Za-z0-9-_.~` (and `/` in a path).
fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (keep_slash && b == b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `Authorization` header value for a request. `headers` are the ones to sign: lower-case
/// names, sorted, values trimmed. `canonical_query` is already encoded and sorted.
#[allow(clippy::too_many_arguments)]
fn authorization(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(String, String)],
    payload_hash: &str,
    amz_datetime: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
) -> String {
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let request =
        format!("{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed}\n{payload_hash}");
    let date = &amz_datetime[..8];
    let scope = format!("{date}/{region}/s3/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_datetime}\n{scope}\n{}", sha256_hex(request.as_bytes()));
    let key = ["aws4_request", "s3", region, date]
        .iter()
        .rev()
        .fold(format!("AWS4{secret_key}").into_bytes(), |k, part| hmac_sha256(&k, part.as_bytes()));
    let signature = hex(&hmac_sha256(&key, to_sign.as_bytes()));
    format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={signature}")
}

struct Answer {
    status: u16,
    body: Vec<u8>,
}

fn parse_answer(raw: &[u8]) -> Result<Answer> {
    let bad = |why: &str| Error::Store(format!("the store sent an answer that is not HTTP/1.1: {why}"));
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| bad("no end of headers"))? + 4;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("no status line"))?;
    let (mut length, mut chunked) = (None, false);
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse::<usize>().ok(),
                "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
                _ => {}
            }
        }
    }
    let rest = &raw[split..];
    let body = if chunked {
        let (mut out, mut at) = (Vec::new(), 0);
        loop {
            let eol = rest[at..].windows(2).position(|w| w == b"\r\n").ok_or_else(|| bad("a cut chunk header"))?;
            let size = std::str::from_utf8(&rest[at..at + eol])
                .ok()
                .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok())
                .ok_or_else(|| bad("a bad chunk size"))?;
            at += eol + 2;
            if size == 0 {
                break out;
            }
            out.extend_from_slice(rest.get(at..at + size).ok_or_else(|| bad("a cut chunk"))?);
            at += size + 2;
        }
    } else if let Some(n) = length {
        rest.get(..n).ok_or_else(|| bad("the body is shorter than its length"))?.to_vec()
    } else {
        rest.to_vec()
    };
    Ok(Answer { status, body })
}

/// The text between `<tag>` and `</tag>`, each time it appears.
fn tags<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(&close) else { break };
        found.push(&after[..end]);
        rest = &after[end + close.len()..];
    }
    found
}

impl S3Store {
    /// A store for `bucket` configured by the environment: `AWS_ACCESS_KEY_ID` and
    /// `AWS_SECRET_ACCESS_KEY` (and `AWS_SESSION_TOKEN` if there is one), `KERNO_S3_REGION` (default
    /// `us-east-1`), `KERNO_S3_ENDPOINT` (default `https://s3.<region>.amazonaws.com`; for R2,
    /// `https://<account>.r2.cloudflarestorage.com` with the region `auto`) and `KERNO_S3_CA_FILE`.
    ///
    /// # Errors
    ///
    /// A missing credential, or an endpoint or CA file that does not load.
    pub fn from_env(bucket: &str) -> Result<Self> {
        Self::new(bucket, config_from(&env)?)
    }

    pub fn new(bucket: &str, config: S3Config) -> Result<Self> {
        check_key(bucket).map_err(|_| Error::Store(format!("{bucket:?} is not a valid bucket name")))?;
        let (scheme_tls, rest) = if let Some(r) = config.endpoint.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = config.endpoint.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(Error::Store(format!("the endpoint {:?} is not an http:// or https:// URL", config.endpoint)));
        };
        let authority = rest.trim_end_matches('/');
        if authority.is_empty() || authority.contains('/') || authority.contains('@') {
            return Err(Error::Store(format!("the endpoint {:?} must be just a host and a port", config.endpoint)));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().map_err(|_| Error::Store(format!("bad port in {authority:?}")))?),
            None => (authority, if scheme_tls { 443 } else { 80 }),
        };
        let tls = scheme_tls.then(|| tls_config(config.ca_file.as_deref())).transpose()?;
        Ok(S3Store { bucket: bucket.to_owned(), scheme_tls, host: host.to_owned(), port, tls, config })
    }

    fn host_header(&self) -> String {
        let default = if self.scheme_tls { 443 } else { 80 };
        if self.port == default { self.host.clone() } else { format!("{}:{}", self.host, self.port) }
    }

    /// One signed request; the answer is read to the end of the connection.
    fn call(&self, method: &str, key: Option<&str>, query: &[(&str, &str)], body: &[u8]) -> Result<Answer> {
        let uri = match key {
            Some(k) => format!("/{}/{}", self.bucket, encode(k, true)),
            None => format!("/{}", self.bucket),
        };
        let mut pairs: Vec<(String, String)> =
            query.iter().map(|(k, v)| (encode(k, false), encode(v, false))).collect();
        pairs.sort();
        let canonical_query = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");

        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let (datetime, _) = amz_date(secs);
        let payload_hash = sha256_hex(body);
        let mut headers = vec![
            ("host".to_owned(), self.host_header()),
            ("x-amz-content-sha256".to_owned(), payload_hash.clone()),
            ("x-amz-date".to_owned(), datetime.clone()),
        ];
        if let Some(token) = &self.config.session_token {
            headers.push(("x-amz-security-token".to_owned(), token.clone()));
        }
        headers.sort();
        let auth = authorization(
            method,
            &uri,
            &canonical_query,
            &headers,
            &payload_hash,
            &datetime,
            &self.config.region,
            &self.config.access_key,
            &self.config.secret_key,
        );
        let target = if canonical_query.is_empty() { uri } else { format!("{uri}?{canonical_query}") };
        let mut head = format!("{method} {target} HTTP/1.1\r\n");
        for (k, v) in &headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str(&format!("Authorization: {auth}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()));

        let io = |e: std::io::Error| Error::Store(format!("{method} on the store at {}: {e}", self.host_header()));
        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(io)?
            .next()
            .ok_or_else(|| Error::Store(format!("{} does not resolve", self.host)))?;
        let tcp = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(io)?;
        tcp.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
        tcp.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
        let mut raw = Vec::new();
        let result = match &self.tls {
            Some(config) => {
                let name = rustls::pki_types::ServerName::try_from(self.host.clone())
                    .map_err(|_| Error::Store(format!("{:?} is not a valid server name", self.host)))?;
                let conn = rustls::ClientConnection::new(Arc::clone(config), name)
                    .map_err(|e| Error::Store(format!("TLS: {e}")))?;
                let mut stream = rustls::StreamOwned::new(conn, tcp);
                stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body)).and_then(|()| {
                    match stream.read_to_end(&mut raw) {
                        // A peer that closes without close_notify has still sent its whole answer.
                        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(0),
                        other => other,
                    }
                })
            }
            None => {
                let mut stream = tcp;
                stream
                    .write_all(head.as_bytes())
                    .and_then(|()| stream.write_all(body))
                    .and_then(|()| stream.read_to_end(&mut raw))
            }
        };
        result.map_err(io)?;
        parse_answer(&raw)
    }

    fn failed(&self, what: &str, answer: &Answer) -> Error {
        let text = String::from_utf8_lossy(&answer.body);
        let code = tags(&text, "Code").first().copied().unwrap_or("");
        let message = tags(&text, "Message").first().copied().unwrap_or("");
        Error::Store(format!("{what}: the store answered {} {code} {message}", answer.status).trim_end().to_owned())
    }
}

fn tls_config(ca_file: Option<&std::path::Path>) -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    let (added, _) = roots.add_parsable_certificates(native.certs);
    if let Some(path) = ca_file {
        for cert in CertificateDer::pem_file_iter(path)
            .map_err(|e| Error::Store(format!("cannot read the CA file {}: {e}", path.display())))?
        {
            let cert = cert.map_err(|e| Error::Store(format!("bad certificate in {}: {e}", path.display())))?;
            roots.add(cert).map_err(|e| Error::Store(format!("bad certificate in {}: {e}", path.display())))?;
        }
    } else if added == 0 {
        return Err(Error::Store(
            "no system root certificates were found: set SSL_CERT_FILE, or KERNO_S3_CA_FILE to a PEM file".into(),
        ));
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Store(format!("TLS: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

impl ObjectStore for S3Store {
    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        check_key(key)?;
        let answer = self.call("PUT", Some(key), &[], data)?;
        if answer.status == 200 { Ok(()) } else { Err(self.failed(&format!("cannot write {key}"), &answer)) }
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        check_key(key)?;
        let answer = self.call("GET", Some(key), &[], &[])?;
        match answer.status {
            200 => Ok(Some(answer.body)),
            404 => Ok(None),
            _ => Err(self.failed(&format!("cannot read {key}"), &answer)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let (mut keys, mut token) = (Vec::new(), None::<String>);
        loop {
            let mut query = vec![("list-type", "2"), ("prefix", prefix)];
            if let Some(t) = &token {
                query.push(("continuation-token", t));
            }
            let answer = self.call("GET", None, &query, &[])?;
            if answer.status != 200 {
                return Err(self.failed(&format!("cannot list {prefix:?}"), &answer));
            }
            let xml = String::from_utf8_lossy(&answer.body).into_owned();
            keys.extend(tags(&xml, "Key").into_iter().map(str::to_owned));
            match (tags(&xml, "IsTruncated").first().copied(), tags(&xml, "NextContinuationToken").first()) {
                (Some("true"), Some(next)) => token = Some(next.replace("&amp;", "&")),
                (Some("true"), None) => return Err(Error::Store("a truncated listing without a next token".into())),
                _ => break,
            }
        }
        keys.sort();
        Ok(keys)
    }

    fn delete(&self, key: &str) -> Result<()> {
        check_key(key)?;
        let answer = self.call("DELETE", Some(key), &[], &[])?;
        // S3 answers 204 for a key that is not there; other stores 404.
        if matches!(answer.status, 200 | 204 | 404) {
            Ok(())
        } else {
            Err(self.failed(&format!("cannot delete {key}"), &answer))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_civil_dates() {
        assert_eq!(amz_date(0), ("19700101T000000Z".into(), "19700101".into()));
        assert_eq!(amz_date(1_369_353_600), ("20130524T000000Z".into(), "20130524".into()));
        // A leap day, and the last second of a year.
        assert_eq!(amz_date(1_709_210_096).0, "20240229T123456Z");
        assert_eq!(amz_date(1_735_689_599).0, "20241231T235959Z");
    }

    #[test]
    fn the_signature_is_the_one_aws_documents() {
        // "Example: GET Object" of the AWS Signature Version 4 documentation (virtual-hosted style,
        // a Range header).
        let headers: Vec<(String, String)> = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            ("x-amz-date", "20130524T000000Z"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        let auth = authorization(
            "GET",
            "/test.txt",
            "",
            &headers,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "20130524T000000Z",
            "us-east-1",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn encoding_keeps_unreserved_characters_and_a_path_slash() {
        assert_eq!(encode("a-b_c.d~e", false), "a-b_c.d~e");
        assert_eq!(encode("wal/00 1+=", false), "wal%2F00%201%2B%3D");
        assert_eq!(encode("wal/001", true), "wal/001");
    }

    #[test]
    fn answers_with_a_length_a_chunked_body_or_neither_are_read() {
        let a = parse_answer(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcdef").unwrap();
        assert_eq!((a.status, a.body.as_slice()), (200, &b"abc"[..]));
        let a =
            parse_answer(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=1\r\nde\r\n0\r\n\r\n")
                .unwrap();
        assert_eq!(a.body, b"abcde");
        let a = parse_answer(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\nrest").unwrap();
        assert_eq!((a.status, a.body.as_slice()), (404, &b"rest"[..]));
        for bad in [&b"nonsense"[..], b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nabc", b"HTTP/1.1 x\r\n\r\n"] {
            assert!(parse_answer(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn the_configuration_comes_from_named_variables_with_defaults() {
        let vars = |pairs: &'static [(&str, &str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
        };
        let c = config_from(&vars(&[("AWS_ACCESS_KEY_ID", "id"), ("AWS_SECRET_ACCESS_KEY", "secret")])).unwrap();
        assert_eq!((c.endpoint.as_str(), c.region.as_str()), ("https://s3.us-east-1.amazonaws.com", "us-east-1"));
        assert_eq!((c.session_token, c.ca_file), (None, None));
        let c = config_from(&vars(&[
            ("AWS_ACCESS_KEY_ID", "id"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
            ("AWS_SESSION_TOKEN", "t"),
            ("KERNO_S3_REGION", "auto"),
            ("KERNO_S3_ENDPOINT", "https://acct.r2.cloudflarestorage.com"),
            ("KERNO_S3_CA_FILE", "/ca.pem"),
        ]))
        .unwrap();
        assert_eq!((c.endpoint.as_str(), c.region.as_str()), ("https://acct.r2.cloudflarestorage.com", "auto"));
        assert_eq!((c.session_token.as_deref(), c.ca_file), (Some("t"), Some(PathBuf::from("/ca.pem"))));
        for (missing, set) in
            [("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"), ("AWS_SECRET_ACCESS_KEY", "AWS_ACCESS_KEY_ID")]
        {
            let only = move |n: &str| (n == set).then(|| "x".to_owned());
            assert_eq!(config_from(&only).err().map(|e| e.to_string()), Some(format!("{missing} is not set")));
        }
    }

    #[test]
    fn an_endpoint_is_a_scheme_a_host_and_maybe_a_port() {
        let with = |endpoint: &str| {
            S3Store::new(
                "bucket",
                S3Config {
                    endpoint: endpoint.into(),
                    region: "auto".into(),
                    access_key: "a".into(),
                    secret_key: "s".into(),
                    session_token: None,
                    ca_file: None,
                },
            )
        };
        assert_eq!(with("http://localhost:9000").unwrap().host_header(), "localhost:9000");
        assert_eq!(with("http://localhost/").unwrap().host_header(), "localhost");
        for bad in ["localhost:9000", "ftp://x", "http://", "http://h/path", "http://u@h", "http://h:port"] {
            assert!(with(bad).is_err(), "{bad}");
        }
        assert!(S3Store::new("bad bucket", config_from(&|_| Some("x".into())).unwrap()).is_err());
    }

    #[test]
    fn tags_are_read_in_order() {
        assert_eq!(tags("<A><Key>x</Key><Key>y/z</Key><B/></A>", "Key"), ["x", "y/z"]);
        assert!(tags("<Key>cut", "Key").is_empty());
    }
}
