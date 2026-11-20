//! A client of the OpenAI-compatible `POST {base_url}/embeddings` endpoint,
//! behind the cargo feature `embed-openai` (off by default). It serves a remote
//! API and a local model alike: only the base URL changes.

use std::fmt;
use std::time::Duration;

use serde_json::{Value, json};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use super::{EmbedError, EmbeddingProvider};
use crate::vector_store::{MAX_DIM, MAX_PROFILE_LEN};

/// Variables read by [`OpenAiCompatible::from_env`].
pub const ENV_BASE_URL: &str = "KERNO_EMBED_BASE_URL";
pub const ENV_MODEL: &str = "KERNO_EMBED_MODEL";
pub const ENV_DIM: &str = "KERNO_EMBED_DIM";
pub const ENV_API_KEY: &str = "KERNO_EMBED_API_KEY";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Texts sent per request: the API refuses very large inputs.
const BATCH: usize = 256;

/// The key is read from the environment when the provider is built and kept
/// in memory only; it is never written to the database, an error or `Debug`.
pub struct OpenAiCompatible {
    agent: ureq::Agent,
    url: String,
    model: String,
    dim: usize,
    api_key: Option<String>,
}

impl fmt::Debug for OpenAiCompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatible")
            .field("url", &self.url)
            .field("model", &self.model)
            .field("dim", &self.dim)
            .field("api_key", &self.api_key.as_ref().map(|_| "<set>"))
            .finish()
    }
}

fn env(name: &str) -> Result<Option<String>, EmbedError> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(Some(v)),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(EmbedError::Provider(format!("{name} is not valid UTF-8"))),
    }
}

fn required(name: &str) -> Result<String, EmbedError> {
    env(name)?.ok_or_else(|| EmbedError::Provider(format!("{name} is not set")))
}

impl OpenAiCompatible {
    /// A provider of `model` at `base_url` (for instance `https://api.openai.com/v1`
    /// or `http://localhost:11434/v1`) whose vectors have `dim` elements: the
    /// model's own dimension. No credential; see [`with_api_key_env`](Self::with_api_key_env).
    pub fn new(base_url: &str, model: &str, dim: usize) -> Result<Self, EmbedError> {
        if dim == 0 || dim > MAX_DIM as usize {
            return Err(EmbedError::Provider(format!("dim {dim} is outside 1..={MAX_DIM}")));
        }
        if model.is_empty() || model.len() > MAX_PROFILE_LEN {
            return Err(EmbedError::Provider(format!("the model name must have 1..={MAX_PROFILE_LEN} bytes")));
        }
        // The platform's TLS and trust store (SecureTransport, OpenSSL, SChannel).
        // ureq panics on its default roots when no webpki roots are compiled in,
        // so the provider and the roots are named here.
        let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build();
        let agent: ureq::Agent =
            ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).tls_config(tls).build().into();
        Ok(OpenAiCompatible {
            agent,
            url: format!("{}/embeddings", base_url.trim_end_matches('/')),
            model: model.to_owned(),
            dim,
            api_key: None,
        })
    }

    /// Reads the bearer token from the environment variable `var`; an unset or
    /// empty variable is an error, so a missing key fails here and not as a 401.
    pub fn with_api_key_env(mut self, var: &str) -> Result<Self, EmbedError> {
        self.api_key = Some(required(var)?);
        Ok(self)
    }

    /// Builds the provider from [`ENV_BASE_URL`] (default OpenAI's), [`ENV_MODEL`]
    /// and [`ENV_DIM`] (both required) and, if set, [`ENV_API_KEY`]. A local
    /// server needs no key.
    pub fn from_env() -> Result<Self, EmbedError> {
        let base = env(ENV_BASE_URL)?.unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        let dim = required(ENV_DIM)?
            .parse::<usize>()
            .map_err(|_| EmbedError::Provider(format!("{ENV_DIM} is not a number")))?;
        let mut p = Self::new(&base, &required(ENV_MODEL)?, dim)?;
        p.api_key = env(ENV_API_KEY)?;
        Ok(p)
    }

    fn request(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut req = self.agent.post(&self.url);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let body = json!({ "model": self.model, "input": texts, "encoding_format": "float" });
        let mut resp = req.send_json(&body).map_err(|e| EmbedError::Provider(format!("request failed: {e}")))?;
        let reply: Value =
            resp.body_mut().read_json().map_err(|e| EmbedError::BadOutput(format!("the reply is not JSON: {e}")))?;
        parse(&reply, texts.len())
    }
}

/// The vectors of a reply, in the order of the inputs (`data[].index`).
fn parse(reply: &Value, want: usize) -> Result<Vec<Vec<f32>>, EmbedError> {
    let bad = |m: &str| EmbedError::BadOutput(m.to_owned());
    let data = reply.get("data").and_then(Value::as_array).ok_or_else(|| bad("the reply has no data array"))?;
    if data.len() != want {
        return Err(EmbedError::BadOutput(format!("{} vectors for {want} texts", data.len())));
    }
    let mut out: Vec<Option<Vec<f32>>> = vec![None; want];
    for item in data {
        let index = item.get("index").and_then(Value::as_u64).ok_or_else(|| bad("an item has no index"))?;
        let slot = out.get_mut(index as usize).ok_or_else(|| bad("an item index is out of range"))?;
        let values = item.get("embedding").and_then(Value::as_array).ok_or_else(|| bad("an item has no embedding"))?;
        let vector = values
            .iter()
            .map(|x| x.as_f64().map(|x| x as f32))
            .collect::<Option<Vec<f32>>>()
            .ok_or_else(|| bad("an embedding has a non-number"))?;
        if slot.replace(vector).is_some() {
            return Err(bad("two items share an index"));
        }
    }
    Ok(out.into_iter().flatten().collect())
}

impl EmbeddingProvider for OpenAiCompatible {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH) {
            out.extend(self.request(batch)?);
        }
        Ok(out)
    }

    fn profile(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn parse_orders_by_index_and_rejects_malformed_replies() {
        let ok = json!({"data": [
            {"index": 1, "embedding": [3.0, 4.0]},
            {"index": 0, "embedding": [1.0, 2.0]},
        ]});
        assert_eq!(parse(&ok, 2).unwrap(), vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
        assert!(parse(&ok, 3).is_err(), "a missing vector");
        assert!(parse(&json!({}), 1).is_err());
        assert!(parse(&json!({"data": [{"index": 5, "embedding": [1.0]}]}), 1).is_err());
        assert!(parse(&json!({"data": [{"index": 0, "embedding": ["x"]}]}), 1).is_err());
        let twice = json!({"data": [{"index": 0, "embedding": [1.0]}, {"index": 0, "embedding": [1.0]}]});
        assert!(parse(&twice, 2).is_err());
    }

    #[test]
    fn construction_is_checked_and_debug_hides_the_key() {
        assert!(OpenAiCompatible::new("http://x/v1", "m", 0).is_err());
        assert!(OpenAiCompatible::new("http://x/v1", "", 8).is_err());
        assert!(OpenAiCompatible::new("http://x/v1", &"m".repeat(65), 8).is_err());
        let mut p = OpenAiCompatible::new("http://x/v1/", "m", 8).unwrap();
        assert_eq!(p.url, "http://x/v1/embeddings");
        let unset = OpenAiCompatible::new("http://x/v1", "m", 8).unwrap();
        assert!(unset.with_api_key_env("KERNO_EMBED_TEST_VAR_THAT_IS_NEVER_SET").is_err());
        p.api_key = Some("sk-not-a-real-key".into());
        assert!(!format!("{p:?}").contains("sk-not-a-real-key"));
    }

    /// An https request builds the TLS connector (ureq panics there if the root
    /// certificates are not named). The server accepts and hangs up, so the
    /// handshake fails: an `Err`, never a panic.
    #[test]
    fn https_fails_cleanly_when_the_handshake_does() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("https://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || drop(listener.accept().unwrap()));
        let p = OpenAiCompatible::new(&base, "tiny", 2).unwrap();
        assert!(matches!(p.embed(&["a"]), Err(EmbedError::Provider(_))));
        server.join().unwrap();
    }

    /// One request against a one-shot local server: the path, the bearer
    /// token and the JSON body are what the endpoint expects.
    #[test]
    fn embed_talks_to_an_openai_compatible_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0_u8; 8192];
            let mut n = 0;
            let request = loop {
                n += s.read(&mut buf[n..]).unwrap();
                let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                if let Some(at) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned))
                        .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                    if n >= at + 4 + len {
                        break text;
                    }
                }
            };
            let body = r#"{"data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]}"#;
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        });
        let mut p = OpenAiCompatible::new(&base, "tiny", 2).unwrap();
        p.api_key = Some("test-token".into());
        let got = p.embed(&["a", "b"]).unwrap();
        assert_eq!(got, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/embeddings "), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer test-token"));
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!((&body["model"], &body["input"]), (&json!("tiny"), &json!(["a", "b"])), "{body}");
    }
}
