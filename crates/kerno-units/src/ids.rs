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
//! * **Text form** `k1_` followed by 26 characters: the digit is the text
//!   version ([`PUBLIC_ID_TEXT_VERSION`], fixed from the first day, so a later
//!   codec can coexist), the body is the 128 bits in RFC 4648 base32, lower
//!   case, without padding. 26 characters carry 130 bits; the last two are
//!   zero and a token with them set is refused (one text per id, so a token is
//!   not malleable). No application prefix: the ADR calls it optional and
//!   nothing needs it yet.
//! * **Validation order** (cheapest first, before any lookup): prefix `k`,
//!   version digits and `_`; version; body length; alphabet; canonical
//!   trailing bits. Each failure is its own [`PublicIdError`].
//! * **Source of randomness**: the operating system's `/dev/urandom`, read
//!   through `std` as `FingerprintKey::generate` does; no dependency is added.
//!   That file exists on Linux, macOS, the BSDs and other Unix systems. On any
//!   other platform (Windows, WASI) opening it fails and
//!   [`ChangeSet::create_public`] returns that error as
//!   `UnitError::Storage(Io)` without creating anything; reading and
//!   resolving existing tokens still work. A read error or a short read is an
//!   error too, and so is a token whose 16 bytes are all equal (a constant
//!   source): it is refused, never retried, so a broken source cannot make
//!   creation spin. Upgrade path: `getrandom` if a non-Unix target is ever
//!   supported.
//! * **Stored records**, both written by [`ChangeSet::create_public`] in the
//!   unit's own transaction and never rewritten (`docs/file-format/units-v3.md`):
//!   * `0x70 ‖ token[16]` → `envelope_version ‖ reserved_flags ‖ unit_id[16]`
//!     (token to unit);
//!   * `0x71 ‖ unit_id[16]` → `envelope_version ‖ reserved_flags ‖ token[16]`
//!     (unit to token, so the token of a unit can be shown again without a
//!     scan).
//!
//!   The value starts with the layer's common envelope (units-v1 §1.1):
//!   `envelope_version` is [`PUBLIC_ID_RECORD_VERSION`] and `reserved_flags`
//!   is `0` today, a byte kept for a later revoked or reissued state. The
//!   value is checked before the key's length, so a later record with another
//!   token length reads as a newer layer (UNIT-7), not as damage. Key lengths
//!   are 17 and value lengths 18; a version of `0` is UNIT-6, above `1` is
//!   UNIT-7, flags other than `0` are UNIT-8, a short or long value is UNIT-9
//!   or UNIT-10 and a key of another length is UNIT-3.
//! * **Layer mark.** The first transaction that writes a token raises the mark
//!   to `layer_version = 3`, as the first derived record raises it to 2, and
//!   nothing lowers it. A build that knows only layers 1 and 2 refuses such a
//!   database whole (UNIT-16) and writes nothing.
//! * **Resolution** reads the token record, then the reverse record of the
//!   unit it names, and refuses a pair that disagrees (`Corruption`).
//!   [`UnitSnapshot::public_id`] does the same from the other end, so neither
//!   call returns a token that does not resolve. Error texts name the token or
//!   the unit, never both.
//! * **Lifecycle.** Deleting a unit removes both records in the same
//!   transaction, so its token then resolves like one never issued.
//!   Retention of old revisions leaves the records of a live unit alone.
//! * **Units without a token** are normal: a unit made by
//!   [`ChangeSet::create`], or written before this module existed, has none
//!   and [`UnitSnapshot::public_id`] returns `None`. The logical export carries
//!   the token of each unit that has one and `import` restores it.
//!   `ponytail:` a token is only issued at creation; reissuing or revoking
//!   one needs a change set that carries a member, which is not built.

use std::fmt;
use std::io::Read;
use std::str::FromStr;

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::codec::ENVELOPE_VERSION;
use crate::db::{ChangeSet, NewUnit, UnitDb, UnitSnapshot, Write, corruption};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::types::UnitId;

/// Key tag of the record `token → unit`.
pub const TAG_PUBLIC_ID: u8 = 0x70;
/// Key tag of the record `unit → token`.
pub const TAG_PUBLIC_ID_OF: u8 = 0x71;
/// The version digit in the text of every token (`k1_...`).
pub const PUBLIC_ID_TEXT_VERSION: u8 = 1;
/// The `envelope_version` of the `0x70`/`0x71` records: the layer's common
/// envelope, which these records share with every other (units-v1 §1.1).
pub const PUBLIC_ID_RECORD_VERSION: u8 = ENVELOPE_VERSION;
/// Length of a record key (tag and 16 bytes) and of a record value (envelope
/// and 16 bytes).
pub(crate) const KEY_LEN: usize = 17;
pub(crate) const VALUE_LEN: usize = 18;

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
            PublicIdError::Version { found } => write!(f, "public id version {found} is not {PUBLIC_ID_TEXT_VERSION}"),
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
        if version != u32::from(PUBLIC_ID_TEXT_VERSION) {
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
    let mut v = vec![PUBLIC_ID_RECORD_VERSION, 0];
    v.extend_from_slice(other);
    v
}

/// The envelope of a record value: version, then flags (units-v1 §1.1).
fn check_envelope(v: &[u8]) -> Result<(), DecodeError> {
    let (Some(&version), Some(&flags)) = (v.first(), v.get(1)) else {
        return Err(DecodeError::new(
            UnitRule::Unit9,
            format!("public id record has {} bytes, fewer than its envelope", v.len()),
        ));
    };
    match version {
        0 => return Err(DecodeError::new(UnitRule::Unit6, "public id record envelope_version 0")),
        PUBLIC_ID_RECORD_VERSION => {}
        n => return Err(DecodeError::new(UnitRule::Unit7, format!("public id record envelope_version {n}"))),
    }
    if flags != 0 {
        return Err(DecodeError::new(UnitRule::Unit8, format!("public id record reserved_flags {flags:#04x}")));
    }
    Ok(())
}

/// The 16 bytes of a record value, after the checks of the module
/// documentation: envelope first, then length.
pub(crate) fn decode_value(v: &[u8]) -> Result<[u8; 16], DecodeError> {
    check_envelope(v)?;
    match v.len() {
        n if n < VALUE_LEN => {
            Err(DecodeError::new(UnitRule::Unit9, format!("public id record has {} bytes after its envelope", n - 2)))
        }
        VALUE_LEN => Ok(v[2..].try_into().unwrap_or([0; 16])),
        n => Err(DecodeError::new(UnitRule::Unit10, format!("public id record has {} trailing bytes", n - VALUE_LEN))),
    }
}

/// Shape check of one `0x70`/`0x71` record, for `verify_units`. The value's
/// envelope is read before the key's length, so a record of a newer shape is
/// a newer layer and not damage.
#[doc(hidden)]
pub fn check_record(key: &[u8], value: &[u8]) -> Result<[u8; 16], DecodeError> {
    check_envelope(value)?;
    if key.len() != KEY_LEN {
        return Err(DecodeError::new(
            UnitRule::Unit3,
            format!("key with tag 0x{:02x} has length {}", key[0], key.len()),
        ));
    }
    decode_value(value)
}

/// A token read from `source`: an error of the read, or a short read, is an
/// error, and so is a token of 16 equal bytes, which only a constant source
/// gives.
fn read_token(mut source: impl Read) -> std::io::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    source.read_exact(&mut bytes)?;
    if bytes.iter().all(|b| *b == bytes[0]) {
        return Err(std::io::Error::other("the random source gave a constant token; refusing to issue one"));
    }
    Ok(bytes)
}

fn os_random() -> std::io::Result<[u8; 16]> {
    read_token(std::fs::File::open("/dev/urandom")?)
}

/// A token nobody holds yet: draws from `random` until `taken` says no. An
/// error of `random` is returned at once, not retried.
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

/// The records of a unit's token, to delete when the unit is deleted. Reads
/// only. A reverse record that does not decode is deleted without its token
/// record, which `verify_units` then reports; a delete does not fail for
/// damage it is removing.
pub(crate) fn plan_removal<R: Fn(&[u8]) -> Result<Option<Vec<u8>>, kerno_core::Error>>(
    get: R,
    unit: UnitId,
) -> Result<Vec<Write>, UnitError> {
    let Some(raw) = get(&unit_key(unit))? else {
        return Ok(Vec::new());
    };
    let mut writes = vec![Write::Delete(unit_key(unit))];
    if let Ok(token) = decode_value(&raw) {
        let token = PublicId(token);
        // Only a token record that names this unit goes with it.
        let names_unit = get(&token_key(&token))?.and_then(|v| decode_value(&v).ok()) == Some(unit.0);
        if names_unit {
            writes.push(Write::Delete(token_key(&token)));
        }
    }
    Ok(writes)
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
        self.write_token(unit, token)?;
        Ok((unit, token))
    }

    /// Writes the two records of `token` for `unit`, which must have none and
    /// whose token must be free in this change set's view. Used by `import`
    /// to restore a token; refused with [`UnitError::PublicIdTaken`] when the
    /// token already names a unit.
    pub(crate) fn restore_public_id(&mut self, unit: UnitId, token: PublicId) -> Result<(), UnitError> {
        let txn = self.txn()?;
        if txn.get(&token_key(&token))?.is_some() || txn.get(&unit_key(unit))?.is_some() {
            return Err(UnitError::PublicIdTaken);
        }
        self.write_token(unit, token)
    }

    fn write_token(&mut self, unit: UnitId, token: PublicId) -> Result<(), UnitError> {
        self.write(&Write::Put(token_key(&token), value(&unit.0)))?;
        self.write(&Write::Put(unit_key(unit), value(&token.0)))?;
        self.token_ops += 1;
        Ok(())
    }

    /// Whether the database this change set sees can hold token records: its
    /// mark is at layer 3, or this change set is about to raise it.
    pub(crate) fn holds_tokens(&self) -> bool {
        self.layer_version >= crate::codec::LAYER_VERSION_PUBLIC_IDS || self.token_ops > 0
    }

    /// The writes that remove the token of `unit`, for its deletion.
    pub(crate) fn plan_token_removal(&self, unit: UnitId) -> Result<Vec<Write>, UnitError> {
        if !self.holds_tokens() {
            return Ok(Vec::new());
        }
        let txn = self.txn()?;
        plan_removal(|k| txn.get(k), unit)
    }
}

impl<E: Engine> UnitSnapshot<E> {
    fn token_of(&self, unit: UnitId) -> Result<Option<PublicId>, UnitError> {
        let Some(raw) = self.snap.get(&unit_key(unit))? else {
            return Ok(None);
        };
        let token =
            PublicId(decode_value(&raw).map_err(|e| crate::db::context(e, &format!("PUBLIC_ID_OF unit {unit}")))?);
        Ok(Some(token))
    }

    fn unit_of(&self, token: &PublicId) -> Result<Option<UnitId>, UnitError> {
        let Some(raw) = self.snap.get(&token_key(token))? else {
            return Ok(None);
        };
        Ok(Some(UnitId(decode_value(&raw).map_err(|e| crate::db::context(e, &format!("PUBLIC_ID {token}")))?)))
    }

    /// The public id issued for `unit`, or `None` for a unit without one. The
    /// token is returned only if its own record names `unit`: a reverse record
    /// without its pair is `Corruption`, as in
    /// [`UnitSnapshot::resolve_public_id`].
    pub fn public_id(&self, unit: UnitId) -> Result<Option<PublicId>, UnitError> {
        let Some(token) = self.token_of(unit)? else {
            return Ok(None);
        };
        if self.unit_of(&token)? != Some(unit) {
            return Err(corruption("a unit's public id record is not matched by the token's own record".into()));
        }
        Ok(Some(token))
    }

    /// The unit `token` names, or `None` if it was never issued (or its unit
    /// was deleted: deleting removes the mapping). A valid token grants
    /// nothing (I-27): the caller still needs its own authorisation to read or
    /// change the unit.
    pub fn resolve_public_id(&self, token: &PublicId) -> Result<Option<UnitId>, UnitError> {
        let Some(unit) = self.unit_of(token)? else {
            return Ok(None);
        };
        // The two records are written together; one without the other is damage.
        if self.token_of(unit)? != Some(*token) {
            return Err(corruption("a public id record names a unit whose own record disagrees".into()));
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

impl PublicId {
    /// The 128 bits of the token, as the stored records and the logical
    /// export hold them.
    pub fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// A token from its 128 bits. Holding one grants nothing (I-27); whether
    /// it was ever issued is the resolver's answer.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        PublicId(bytes)
    }
}

/// The two records of a pair, `[token to unit, unit to token]`, each as
/// `(key, value)`: the bytes `create_public` writes (`units-v3.md`).
#[doc(hidden)]
pub fn records(unit: UnitId, token: &PublicId) -> [(Vec<u8>, Vec<u8>); 2] {
    [(token_key(token), value(&unit.0)), (unit_key(unit), value(&token.0))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varied(n: u8) -> [u8; 16] {
        std::array::from_fn(|i| n.wrapping_add(i as u8))
    }

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
                Ok(varied(n))
            },
            |t| {
                draws += 1;
                Ok(t.0[0] < 3)
            },
        )
        .unwrap();
        assert_eq!((got, draws), (PublicId(varied(3)), 3));
        let mut taken_draws = 0;
        let stuck = mint(
            || {
                taken_draws += 1;
                Ok(varied(9))
            },
            |_| Ok(true),
        );
        assert!(matches!(stuck, Err(UnitError::Storage(_))));
        assert_eq!(taken_draws, MINT_ATTEMPTS, "gives up after the attempts, not before and not never");
    }

    #[test]
    fn a_random_source_that_fails_is_an_error_at_once() {
        let mut draws = 0;
        let got = mint(
            || {
                draws += 1;
                Err(std::io::Error::other("no source"))
            },
            |_| Ok(false),
        );
        assert!(matches!(got, Err(UnitError::Storage(kerno_core::Error::Io(_)))));
        assert_eq!(draws, 1, "an error of the source is not retried");
    }

    #[test]
    fn short_reads_read_errors_and_constant_tokens_are_refused() {
        let short = read_token(&[1u8, 2, 3][..]).unwrap_err();
        assert_eq!(short.kind(), std::io::ErrorKind::UnexpectedEof);
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("read failed"))
            }
        }
        assert!(read_token(Broken).is_err());
        for b in [0u8, 7, 255] {
            let e = read_token(&[b; 16][..]).unwrap_err();
            assert!(e.to_string().contains("constant"), "{e}");
        }
        // One differing byte is enough to be accepted.
        let mut almost = [5u8; 16];
        almost[15] = 6;
        assert_eq!(read_token(&almost[..]).unwrap(), almost);
        // The real source works on this platform.
        assert!(os_random().is_ok());
    }

    #[test]
    fn record_value_checks_the_envelope_before_the_length() {
        let good = value(&varied(1));
        assert_eq!(decode_value(&good).unwrap(), varied(1));
        // A newer envelope with another length is UNIT-7, not a length error.
        let mut newer = vec![2, 0];
        newer.extend_from_slice(&[0u8; 20]);
        assert_eq!(decode_value(&newer).unwrap_err().rule, UnitRule::Unit7);
        let mut key = vec![TAG_PUBLIC_ID];
        key.extend_from_slice(&[0u8; 15]);
        assert_eq!(check_record(&key, &newer).unwrap_err().rule, UnitRule::Unit7);
        assert_eq!(check_record(&key, &good).unwrap_err().rule, UnitRule::Unit3);
    }
}
