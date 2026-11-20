//! API tokens: the tokens file, the SHA-256 the server keeps of each token, and the `Bearer`
//! credential a request carries.
//!
//! The server never holds a token, only its hash: a leaked tokens file does not let anyone in.
//! A token names its tenant, so a request can reach only the database of its own tenant.

use std::collections::HashMap;
use std::path::Path;

use crate::api::from_hex;

/// The fewest characters `kerno-server hash-token` accepts in a token.
pub const MIN_TOKEN_LEN: usize = 32;

/// The SHA-256 of `token`.
pub fn hash(token: &str) -> [u8; 32] {
    let digest = ring::digest::digest(&ring::digest::SHA256, token.as_bytes());
    let mut out = [0; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// `hash(token)` as 64 lowercase hex digits, the form of the tokens file.
pub fn hash_hex(token: &str) -> String {
    hash(token).iter().map(|b| format!("{b:02x}")).collect()
}

/// A tenant name is its directory name under `--data-dir`, so it is only `[a-z0-9_-]{1,64}`:
/// no `/`, no `..`, nothing a filesystem treats specially.
pub fn valid_tenant(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// The token of an `Authorization: Bearer <token>` value. The scheme is case-insensitive.
pub fn bearer(header: Option<&[u8]>) -> Option<&str> {
    let value = std::str::from_utf8(header?).ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// Which tenant each token hash belongs to.
#[derive(Debug)]
pub struct Tokens(HashMap<[u8; 32], String>);

impl Tokens {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the tokens file {}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("tokens file {}: {e}", path.display()))
    }

    /// One `<sha256 hex> <tenant>` per line; blank lines and lines starting with `#` are skipped.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut tokens = HashMap::new();
        for (n, line) in text.lines().enumerate().map(|(i, l)| (i + 1, l.trim())) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [digest, tenant] = fields[..] else {
                return Err(format!("line {n}: expected `<sha256 hex> <tenant>`"));
            };
            let digest: [u8; 32] = from_hex(digest)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| format!("line {n}: the hash must be 64 hex digits (see `kerno-server hash-token`)"))?;
            if !valid_tenant(tenant) {
                return Err(format!("line {n}: tenant {tenant:?} is not 1 to 64 of a-z, 0-9, _ and -"));
            }
            if tokens.insert(digest, tenant.to_owned()).is_some() {
                return Err(format!("line {n}: this hash is already listed"));
            }
        }
        if tokens.is_empty() {
            return Err("no tokens: nobody could use the server".into());
        }
        Ok(Tokens(tokens))
    }

    /// The tenant of `token`, if the token is listed.
    pub fn tenant_of(&self, token: &str) -> Option<&str> {
        self.0.get(&hash(token)).map(String::as_str)
    }

    /// Every tenant named, each once.
    pub fn tenants(&self) -> std::collections::BTreeSet<&str> {
        self.0.values().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_sha256_in_lowercase_hex() {
        // `printf abc | sha256sum`
        assert_eq!(hash_hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_bearer_credential_is_read_and_anything_else_is_not() {
        assert_eq!(bearer(Some(b"Bearer abc")), Some("abc"));
        assert_eq!(bearer(Some(b"bearer  abc ")), Some("abc"));
        assert_eq!(bearer(Some(b"Basic abc")), None);
        assert_eq!(bearer(Some(b"Bearer ")), None);
        assert_eq!(bearer(Some(b"Bearer")), None);
        assert_eq!(bearer(Some(b"Bearer \xff")), None);
        assert_eq!(bearer(None), None);
    }

    #[test]
    fn tenant_names_cannot_leave_the_data_directory() {
        for good in ["a", "acme", "acme-2_x", &"a".repeat(64)] {
            assert!(valid_tenant(good), "{good}");
        }
        for bad in ["", "..", ".", "a/b", "A", "a b", "é", &"a".repeat(65)] {
            assert!(!valid_tenant(bad), "{bad}");
        }
    }

    #[test]
    fn the_tokens_file_maps_hashes_to_tenants() {
        let text =
            format!("# comment\n\n{} acme\n  {}   globex  \n{} acme\n", hash_hex("t1"), hash_hex("t2"), hash_hex("t3"));
        let tokens = Tokens::parse(&text).unwrap();
        assert_eq!(tokens.tenant_of("t1"), Some("acme"));
        assert_eq!(tokens.tenant_of("t2"), Some("globex"));
        assert_eq!(tokens.tenant_of("t3"), Some("acme"));
        assert_eq!(tokens.tenant_of("t4"), None);
        // The hash is the key, not the token.
        assert_eq!(tokens.tenant_of(&hash_hex("t1")), None);
        assert_eq!(tokens.tenants().into_iter().collect::<Vec<_>>(), ["acme", "globex"]);
    }

    #[test]
    fn a_bad_tokens_file_names_the_line() {
        let h = hash_hex("t");
        let err = |text: &str| Tokens::parse(text).unwrap_err();
        assert!(err(&format!("{h}\n")).starts_with("line 1: expected"));
        assert!(err(&format!("{h} a b\n")).starts_with("line 1: expected"));
        assert!(err("# c\nabcd acme\n").starts_with("line 2: the hash must be 64 hex digits"));
        assert!(err(&format!("{}zz acme\n", &h[..62])).starts_with("line 1: the hash"));
        assert!(err(&format!("{h} ../etc\n")).starts_with("line 1: tenant \"../etc\""));
        assert!(err(&format!("{h} a\n{h} b\n")).starts_with("line 2: this hash is already listed"));
        assert!(err("# only a comment\n").starts_with("no tokens"));
    }
}
