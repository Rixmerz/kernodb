//! Public identifiers (ADR-0005 option 1, invariant I-27, package E9-02).
//!
//! A [`PublicId`] is the opaque text a caller hands to the outside world in
//! place of a [`UnitId`]. It is 128 random bits drawn from the operating
//! system once, when the unit is created: it is not derived from the unit id,
//! the clock or the creation order, so it reveals none of them. It is a
//! *name*, never a capability: resolving a token only says which unit it
//! names, and every operation on that unit is authorised by the layer above
//! (I-27). A token that parses but was never issued resolves to `None`.
//!
//! # Choices the ADR leaves open
//!
//! * **Text form** `k1_` followed by 26 characters: the digit is the codec
//!   version (fixed from the first day, so a later codec can coexist), the
//!   body is the 128 bits in RFC 4648 base32, lower case, without padding.
//!   26 characters carry 130 bits; the last two are zero and a token with
//!   them set is refused (one text per id, so a token is not malleable). No
//!   application prefix: the ADR calls it optional and nothing needs it yet.
//! * **Validation order** (cheapest first, before any lookup): prefix `k`,
//!   version digits and `_`; version; body length; alphabet; canonical
//!   trailing bits. Each failure is its own [`PublicIdError`].
//! * **Source of randomness**: `/dev/urandom`, as `FingerprintKey::generate`
//!   does; no dependency is added, so this reads only on Unix. Upgrade path:
//!   `getrandom` if a non-Unix target is ever supported.
//! * **Stored records**, both written by [`ChangeSet::create_public`] in the
//!   unit's own transaction and never rewritten:
//!   * `0x70 ‖ token[16]` → `version[1] ‖ unit_id[16]` (token to unit);
//!   * `0x71 ‖ unit_id[16]` → `version[1] ‖ token[16]` (unit to token, so the
//!     token of a unit can be shown again without a scan).
//!
//!   `version` is `1` ([`PUBLIC_ID_VERSION`]). Key lengths are 17, value
//!   lengths are 17; a version of `0` is UNIT-6, above `1` is UNIT-7, a short
//!   or long value is UNIT-9 or UNIT-10 and a key of another length is
//!   UNIT-3. The tags sit outside [`crate::codec::Key`] like the persisted
//!   derived data (`0x60..=0x62`): `verify_units` checks their shape. An
//!   older build, which does not know `0x70` and `0x71`, reports them as
//!   unknown keys of a newer layer (UNIT-2); the layer mark is not raised.
//! * **Resolution** reads the token record, then the reverse record of the
//!   unit it names, and refuses a pair that disagrees (`Corruption`).
//! * **Units without a token** are normal: a unit made by
//!   [`ChangeSet::create`] or by `import`, or written before this module
//!   existed, has none and [`UnitSnapshot::public_id`] returns `None`. The
//!   logical export does not carry tokens yet (E9-04 leaves them out).
//!   `ponytail:` a token is only issued at creation; reissuing or revoking
//!   one needs a change set that carries a member, which is not built.

use std::fmt;
use std::io::Read;
use std::str::FromStr;

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::db::{ChangeSet, NewUnit, UnitDb, UnitSnapshot, Write, corruption};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::types::UnitId;

/// Key tag of the record `token → unit`.
pub const TAG_PUBLIC_ID: u8 = 0x70;
/// Key tag of the record `unit → token`.
pub const TAG_PUBLIC_ID_OF: u8 = 0x71;
/// The codec version written in every record and in the text of every token.
pub const PUBLIC_ID_VERSION: u8 = 1;

const PREFIX: &str = "k1_";
const BODY_LEN: usize = 26;
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
/// Draws of a fresh token before giving up. A collision of 128 random bits
/// does not happen; repeated ones mean the random source is broken.
const MINT_ATTEMPTS: usize = 4;

/// An opaque public identifier of one unit. See the [module
/// documentation](self).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicId([u8; 16]);

/// Why a text is not a valid [`PublicId`]. Nothing here says whether the
/// token was ever issued: that is the resolver's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicIdError {
    /// Does not start with `k`, digits and `_`.
    Prefix,
    /// A well-formed prefix with a version this build does not read.
    Version { found: u32 },
    /// The body is not 26 characters.
    Length { found: usize },
    /// A character outside `a-z2-7`, at this byte offset of the text.
    Alphabet { at: usize },
    /// The two spare bits of the last character are not zero.
    NonCanonical,
}

impl fmt::Display for PublicIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PublicIdError::Prefix => f.write_str("public id does not start with k<version>_"),
            PublicIdError::Version { found } => write!(f, "public id version {found} is not {PUBLIC_ID_VERSION}"),
            PublicIdError::Length { found } => write!(f, "public id body has {found} characters, not {BODY_LEN}"),
            PublicIdError::Alphabet { at } => write!(f, "public id has a character outside a-z2-7 at byte {at}"),
            PublicIdError::NonCanonical => f.write_str("public id has non-zero spare bits"),
        }
    }
}

impl std::error::Error for PublicIdError {}

impl fmt::Display for PublicId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::with_capacity(PREFIX.len() + BODY_LEN);
        out.push_str(PREFIX);
        let (mut acc, mut bits) = (0u16, 0u32);
        for &b in &self.0 {
            acc = (acc << 8) | u16::from(b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(char::from(ALPHABET[usize::from((acc >> bits) & 31)]));
            }
        }
        if bits > 0 {
            out.push(char::from(ALPHABET[usize::from((acc << (5 - bits)) & 31)]));
        }
        f.write_str(&out)
    }
}

impl fmt::Debug for PublicId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicId({self})")
    }
}

impl FromStr for PublicId {
    type Err = PublicIdError;

    fn from_str(text: &str) -> Result<Self, PublicIdError> {
        let rest = text.strip_prefix('k').ok_or(PublicIdError::Prefix)?;
        let (digits, body) = rest.split_once('_').ok_or(PublicIdError::Prefix)?;
        // A leading zero would give a second text to version 1.
        if digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || (digits.len() > 1 && digits.starts_with('0'))
        {
            return Err(PublicIdError::Prefix);
        }
        // More than nine digits cannot be a version this codec ever had.
        let version = digits.parse::<u32>().unwrap_or(u32::MAX);
        if version != u32::from(PUBLIC_ID_VERSION) {
            return Err(PublicIdError::Version { found: version });
        }
        if body.len() != BODY_LEN {
            return Err(PublicIdError::Length { found: body.len() });
        }
        let offset = text.len() - body.len();
        let (mut out, mut at) = ([0u8; 16], 0usize);
        let (mut acc, mut bits) = (0u16, 0u32);
        for (i, c) in body.bytes().enumerate() {
            let v = ALPHABET.iter().position(|&a| a == c).ok_or(PublicIdError::Alphabet { at: offset + i })?;
            acc = (acc << 5) | v as u16;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                out[at] = (acc >> bits) as u8;
                at += 1;
            }
        }
        // 26 characters hold 130 bits: 128 are the id, the last 2 must be 0.
        if acc & ((1 << bits) - 1) != 0 {
            return Err(PublicIdError::NonCanonical);
        }
        Ok(PublicId(out))
    }
}

// ------------------------------------------------------------- records

fn token_key(token: &PublicId) -> Vec<u8> {
    let mut k = vec![TAG_PUBLIC_ID];
    k.extend_from_slice(&token.0);
    k
}

fn unit_key(unit: UnitId) -> Vec<u8> {
    let mut k = vec![TAG_PUBLIC_ID_OF];
    k.extend_from_slice(&unit.0);
    k
}

fn value(other: &[u8; 16]) -> Vec<u8> {
    let mut v = vec![PUBLIC_ID_VERSION];
    v.extend_from_slice(other);
    v
}

/// The 16 bytes of a record value, after the checks of the module
/// documentation.
fn decode_value(v: &[u8]) -> Result<[u8; 16], DecodeError> {
    let Some((&version, other)) = v.split_first() else {
        return Err(DecodeError::new(UnitRule::Unit9, "public id record is empty"));
    };
    match version {
        0 => return Err(DecodeError::new(UnitRule::Unit6, "public id record version 0")),
        PUBLIC_ID_VERSION => {}
        n => return Err(DecodeError::new(UnitRule::Unit7, format!("public id record version {n}"))),
    }
    match other.len() {
        n if n < 16 => {
            Err(DecodeError::new(UnitRule::Unit9, format!("public id record has {n} bytes after its version")))
        }
        16 => Ok(other.try_into().unwrap_or([0; 16])),
        n => Err(DecodeError::new(UnitRule::Unit10, format!("public id record has {} trailing bytes", n - 16))),
    }
}

/// Shape check of one `0x70`/`0x71` pair, for `verify_units`.
pub(crate) fn check_record(key: &[u8], value: &[u8]) -> Result<(), DecodeError> {
    if key.len() != 17 {
        return Err(DecodeError::new(
            UnitRule::Unit3,
            format!("key with tag 0x{:02x} has length {}", key[0], key.len()),
        ));
    }
    decode_value(value).map(|_| ())
}

fn os_random() -> std::io::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// A token nobody holds yet: draws from `random` until `taken` says no.
fn mint(
    mut random: impl FnMut() -> std::io::Result<[u8; 16]>,
    mut taken: impl FnMut(&PublicId) -> Result<bool, UnitError>,
) -> Result<PublicId, UnitError> {
    for _ in 0..MINT_ATTEMPTS {
        let token = PublicId(random().map_err(|e| UnitError::Storage(kerno_core::Error::Io(e)))?);
        if !taken(&token)? {
            return Ok(token);
        }
    }
    Err(UnitError::Storage(kerno_core::Error::Io(std::io::Error::other(
        "the random source repeated a public id; refusing to issue one",
    ))))
}

// ------------------------------------------------------------- API

impl<'db, E: TxnEngine + 'db> ChangeSet<'db, E> {
    /// [`ChangeSet::create`] plus a fresh [`PublicId`] for the new unit. The
    /// token and its two records are written in this change set, so they
    /// commit or vanish with the unit. Nothing is created if the random source
    /// fails. The token is readable by [`UnitSnapshot::public_id`] after the
    /// commit.
    pub fn create_public(&mut self, new: NewUnit) -> Result<(UnitId, PublicId), UnitError> {
        self.txn()?;
        let token = mint(os_random, |t| Ok(self.txn()?.get(&token_key(t))?.is_some()))?;
        let unit = self.create(new)?;
        self.write(&Write::Put(token_key(&token), value(&unit.0)))?;
        self.write(&Write::Put(unit_key(unit), value(&token.0)))?;
        Ok((unit, token))
    }
}

impl<E: Engine> UnitSnapshot<E> {
    /// The public id issued for `unit`, or `None` for a unit without one.
    pub fn public_id(&self, unit: UnitId) -> Result<Option<PublicId>, UnitError> {
        let Some(raw) = self.snap.get(&unit_key(unit))? else {
            return Ok(None);
        };
        let token =
            PublicId(decode_value(&raw).map_err(|e| crate::db::context(e, &format!("PUBLIC_ID_OF unit {unit}")))?);
        Ok(Some(token))
    }

    /// The unit `token` names, or `None` if it was never issued. A valid token
    /// grants nothing (I-27): the caller still needs its own authorisation to
    /// read or change the unit. A deleted unit still resolves; its state is
    /// [`UnitSnapshot::get_unit`]'s to report.
    pub fn resolve_public_id(&self, token: &PublicId) -> Result<Option<UnitId>, UnitError> {
        let Some(raw) = self.snap.get(&token_key(token))? else {
            return Ok(None);
        };
        let unit = UnitId(decode_value(&raw).map_err(|e| crate::db::context(e, &format!("PUBLIC_ID {token}")))?);
        // The two records are written together; one without the other is damage.
        if self.public_id(unit)? != Some(*token) {
            return Err(corruption(format!("public id {token} names unit {unit}, whose own record disagrees")));
        }
        Ok(Some(unit))
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// [`UnitSnapshot::public_id`] at the latest commit.
    pub fn public_id(&self, unit: UnitId) -> Result<Option<PublicId>, UnitError> {
        self.snapshot()?.public_id(unit)
    }

    /// [`UnitSnapshot::resolve_public_id`] at the latest commit.
    pub fn resolve_public_id(&self, token: &PublicId) -> Result<Option<UnitId>, UnitError> {
        self.snapshot()?.resolve_public_id(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_has_the_documented_shape() {
        for seed in 0u8..=255 {
            let bytes: [u8; 16] = std::array::from_fn(|i| seed.wrapping_mul(31).wrapping_add(i as u8 * 7));
            let id = PublicId(bytes);
            let text = id.to_string();
            assert_eq!(text.len(), 29);
            assert!(text.starts_with("k1_"));
            assert_eq!(text.parse::<PublicId>(), Ok(id));
        }
        assert_eq!(PublicId([0; 16]).to_string(), "k1_aaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(PublicId([255; 16]).to_string(), "k1_77777777777777777777777774");
    }

    #[test]
    fn mint_redraws_a_taken_token_and_gives_up_after_the_attempts() {
        let mut n = 0u8;
        let mut draws = 0;
        let got = mint(
            || {
                n += 1;
                Ok([n; 16])
            },
            |t| {
                draws += 1;
                Ok(t.0[0] < 3)
            },
        )
        .unwrap();
        assert_eq!((got, draws), (PublicId([3; 16]), 3));
        let stuck = mint(|| Ok([9; 16]), |_| Ok(true));
        assert!(matches!(stuck, Err(UnitError::Storage(_))));
    }
}
