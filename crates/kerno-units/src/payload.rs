//! Integrity check of a revision's payload (`units-v1.md` §3.4, §4, §5.1).
//!
//! A `REV_PAYLOAD` value is opaque bytes; its length and content are fixed by
//! the `payload_len`, `payload_codec`, `payload_present` and `content_hash`
//! fields of the matching `REV_META`. [`verify_payload`] is the only way to
//! obtain a [`VerifiedPayload`], so a payload that fails any of those checks
//! cannot reach the rest of the layer as valid data.
//!
//! The check is pure: it reads its arguments and nothing else, never writes,
//! and returns the same error for the same input.

use std::fmt;

/// Largest payload the format admits (UNIT-22).
pub const MAX_PAYLOAD_LEN: u32 = 1 << 26;

/// The only payload codec of layer version 1: opaque bytes (UNIT-13).
pub const PAYLOAD_CODEC_OPAQUE: u8 = 0;

/// `ContentHash.alg` (`units-v1.md` §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashAlg {
    /// CRC-32C, provisional and without cryptographic guarantee (`alg = 0`).
    Crc32cProvisional,
    /// BLAKE3-256 (`alg = 1`). What writers of this layer produce.
    Blake3,
}

impl HashAlg {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(HashAlg::Crc32cProvisional),
            1 => Some(HashAlg::Blake3),
            _ => None,
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            HashAlg::Crc32cProvisional => 0,
            HashAlg::Blake3 => 1,
        }
    }
}

/// A `ContentHash` field: algorithm and 32-byte digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentHash {
    pub alg: HashAlg,
    pub digest: [u8; 32],
}

impl ContentHash {
    /// Hash of `codec ‖ payload` with `alg`, as a writer stores it.
    pub fn compute(alg: HashAlg, codec: u8, payload: &[u8]) -> Self {
        let digest = match alg {
            HashAlg::Blake3 => {
                let mut h = blake3::Hasher::new();
                h.update(&[codec]);
                h.update(payload);
                *h.finalize().as_bytes()
            }
            HashAlg::Crc32cProvisional => {
                let crc = crc32c::crc32c_append(crc32c::crc32c(&[codec]), payload);
                let mut d = [0u8; 32];
                d[..4].copy_from_slice(&crc.to_be_bytes());
                d
            }
        };
        ContentHash { alg, digest }
    }

    /// BLAKE3-256 of `codec ‖ payload`.
    pub fn blake3(codec: u8, payload: &[u8]) -> Self {
        Self::compute(HashAlg::Blake3, codec, payload)
    }
}

/// What a `REV_META` declares about its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayloadDecl {
    pub content_hash: ContentHash,
    pub payload_len: u32,
    pub payload_codec: u8,
    pub payload_present: bool,
}

/// The payload rules of `units-v1.md` §5.1, plus the field rules they repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PayloadRule {
    /// `payload_codec ≠ 0`.
    Unit13Codec,
    /// `payload_len > 2^26`.
    Unit22,
    /// `payload_present` disagrees with whether `REV_PAYLOAD` exists, or
    /// `payload_present = 0` with `payload_len ≠ 0`.
    Unit26,
    /// The `REV_PAYLOAD` length differs from `payload_len` (truncated or extended).
    Unit27,
    /// The hash of `codec ‖ payload` differs from `content_hash`.
    Unit28,
}

impl PayloadRule {
    pub fn id(self) -> &'static str {
        match self {
            PayloadRule::Unit13Codec => "UNIT-13",
            PayloadRule::Unit22 => "UNIT-22",
            PayloadRule::Unit26 => "UNIT-26",
            PayloadRule::Unit27 => "UNIT-27",
            PayloadRule::Unit28 => "UNIT-28",
        }
    }
}

/// A rejected payload. Always `Corruption`: nothing was written, and reading
/// the same bytes again gives the same error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadError {
    pub rule: PayloadRule,
    pub detail: String,
}

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.rule.id(), self.detail)
    }
}

impl std::error::Error for PayloadError {}

impl From<PayloadError> for kerno_core::Error {
    fn from(e: PayloadError) -> Self {
        kerno_core::Error::Corruption(e.to_string())
    }
}

/// A payload that passed [`verify_payload`]. It cannot be built otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifiedPayload<'a> {
    bytes: Option<&'a [u8]>,
}

impl<'a> VerifiedPayload<'a> {
    /// The payload bytes, or `None` when the revision carries none.
    pub fn bytes(&self) -> Option<&'a [u8]> {
        self.bytes
    }
}

fn err(rule: PayloadRule, detail: String) -> PayloadError {
    PayloadError { rule, detail }
}

/// Checks a stored `REV_PAYLOAD` value (`None` when the key is absent)
/// against its `REV_META` declaration, rejecting the first rule broken in
/// this order: UNIT-13 (codec) → UNIT-22 → UNIT-26 → UNIT-27 → UNIT-28.
/// The length is compared before hashing, so a truncated or oversized value
/// is never hashed.
pub fn verify_payload<'a>(decl: &PayloadDecl, stored: Option<&'a [u8]>) -> Result<VerifiedPayload<'a>, PayloadError> {
    if decl.payload_codec != PAYLOAD_CODEC_OPAQUE {
        return Err(err(PayloadRule::Unit13Codec, format!("payload_codec {} is not 0", decl.payload_codec)));
    }
    if decl.payload_len > MAX_PAYLOAD_LEN {
        return Err(err(PayloadRule::Unit22, format!("payload_len {} exceeds {MAX_PAYLOAD_LEN}", decl.payload_len)));
    }
    let bytes = match (decl.payload_present, stored) {
        (false, None) if decl.payload_len == 0 => return Ok(VerifiedPayload { bytes: None }),
        (false, None) => {
            return Err(err(PayloadRule::Unit26, format!("payload_present = 0 with payload_len {}", decl.payload_len)));
        }
        (false, Some(_)) => {
            return Err(err(PayloadRule::Unit26, "REV_PAYLOAD exists but payload_present = 0".into()));
        }
        (true, None) => {
            return Err(err(PayloadRule::Unit26, "payload_present = 1 but REV_PAYLOAD is missing".into()));
        }
        (true, Some(b)) => b,
    };
    if bytes.len() as u64 != u64::from(decl.payload_len) {
        return Err(err(
            PayloadRule::Unit27,
            format!("REV_PAYLOAD holds {} bytes, payload_len declares {}", bytes.len(), decl.payload_len),
        ));
    }
    let actual = ContentHash::compute(decl.content_hash.alg, decl.payload_codec, bytes);
    if actual != decl.content_hash {
        return Err(err(
            PayloadRule::Unit28,
            format!("content_hash (alg {}) does not match the payload", decl.content_hash.alg.to_byte()),
        ));
    }
    Ok(VerifiedPayload { bytes: Some(bytes) })
}
