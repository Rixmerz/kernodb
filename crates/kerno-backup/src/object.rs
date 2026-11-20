//! What a backup object is: a body behind a magic, a version and a kind, closed by the BLAKE3
//! digest of everything before it. A flipped or missing byte anywhere is [`Error::Corrupt`], so a
//! restore fails instead of producing a database with something missing.

use crate::error::{Error, Result};

const MAGIC: [u8; 8] = *b"KERNOBAK";
const VERSION: u32 = 1;
const DIGEST: usize = 32;
const HEAD: usize = 8 + 4 + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A page of the keys and values of a base backup.
    BasePart = 1,
    /// The record that completes a base backup: it exists only once every part does.
    BaseManifest = 2,
    /// Consecutive commits read from the write-ahead logs.
    WalSegment = 3,
}

pub fn seal(kind: Kind, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEAD + body.len() + DIGEST);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.push(kind as u8);
    out.extend_from_slice(body);
    let digest = blake3::hash(&out);
    out.extend_from_slice(digest.as_bytes());
    out
}

/// The body of `bytes`, checked to be an object of `kind`; `name` is for the error.
pub fn unseal<'a>(kind: Kind, bytes: &'a [u8], name: &str) -> Result<&'a [u8]> {
    let corrupt = |why: &str| Error::Corrupt(format!("{name}: {why}"));
    if bytes.len() < HEAD + DIGEST {
        return Err(corrupt("shorter than an object header"));
    }
    let (signed, digest) = bytes.split_at(bytes.len() - DIGEST);
    if blake3::hash(signed).as_bytes() != digest {
        return Err(corrupt("the digest does not match: the object is damaged"));
    }
    if signed[..8] != MAGIC {
        return Err(corrupt("not a Kerno backup object"));
    }
    let version = u32::from_le_bytes([signed[8], signed[9], signed[10], signed[11]]);
    if version != VERSION {
        return Err(corrupt(&format!("object version {version}, this build reads {VERSION}")));
    }
    if signed[12] != kind as u8 {
        return Err(corrupt("an object of another kind"));
    }
    Ok(&signed[HEAD..])
}

/// A cursor over the body of an object; every read is bounds-checked.
pub struct Cursor<'a> {
    bytes: &'a [u8],
    name: &'a str,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8], name: &'a str) -> Self {
        Cursor { bytes, name }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < n {
            return Err(Error::Corrupt(format!("{}: the body ends early", self.name)));
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_object_opens_and_any_damage_is_refused() {
        let sealed = seal(Kind::BasePart, b"hello");
        assert_eq!(unseal(Kind::BasePart, &sealed, "x").unwrap(), b"hello");
        // Every single flipped byte, and every truncation, is an error.
        for i in 0..sealed.len() {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(matches!(unseal(Kind::BasePart, &bad, "x"), Err(Error::Corrupt(_))), "byte {i}");
        }
        for n in 0..sealed.len() {
            assert!(matches!(unseal(Kind::BasePart, &sealed[..n], "x"), Err(Error::Corrupt(_))), "length {n}");
        }
        let err = unseal(Kind::WalSegment, &sealed, "wal/1").unwrap_err().to_string();
        assert_eq!(err, "wal/1: an object of another kind");
    }

    #[test]
    fn a_newer_version_is_named() {
        let mut sealed = seal(Kind::BasePart, b"x");
        sealed[8] = 2;
        let n = sealed.len() - DIGEST;
        let digest = blake3::hash(&sealed[..n]);
        sealed[n..].copy_from_slice(digest.as_bytes());
        let err = unseal(Kind::BasePart, &sealed, "x").unwrap_err().to_string();
        assert_eq!(err, "x: object version 2, this build reads 1");
    }

    #[test]
    fn a_cursor_never_reads_past_the_end() {
        let mut c = Cursor::new(&[1, 0, 0, 0, 9], "x");
        assert_eq!(c.u32().unwrap(), 1);
        assert!(c.u32().is_err());
        assert_eq!(c.take(1).unwrap(), [9]);
        assert!(c.is_empty());
    }
}
