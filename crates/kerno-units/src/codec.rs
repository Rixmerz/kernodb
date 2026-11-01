//! Production codecs of the layer's keys and values (`units-v1.md` §1–§4).
//!
//! Decoding follows `units-v1.md` §6: key rules (UNIT-1..5) → envelope
//! (UNIT-6..8) → fields in byte order, each with its own checks as soon as
//! it is read → trailing bytes (UNIT-10) → rules between fields
//! (UNIT-17..21). The first rule broken is returned. The reader never
//! panics, never reads past the buffer, and compares a length prefix with
//! the field maximum (UNIT-11) and then with the remaining bytes (UNIT-9)
//! before touching the body, so no allocation follows an unchecked length.
//!
//! Encoders only produce records a decoder accepts: each encoded value is
//! decoded again before it is returned, and a record that would break a rule
//! is refused with `UnitError::InvalidArgument` naming the rule.

use crate::error::{DecodeError, UnitError, UnitRule};
use crate::payload::{ContentHash, HashAlg, MAX_PAYLOAD_LEN, PAYLOAD_CODEC_OPAQUE, PayloadDecl};
use crate::types::{CommitSeq, HistoryPolicy, Lifecycle, Purpose, RevNo, RevisionKind, Timestamp, UnitId};

/// Key tags (`units-v1.md` §2).
pub const TAG_LAYER_META: u8 = 0x01;
pub const TAG_UNIT_HEAD: u8 = 0x10;
pub const TAG_REV_META: u8 = 0x20;
pub const TAG_REV_PAYLOAD: u8 = 0x21;
pub const TAG_CHANGESET: u8 = 0x30;
pub const TAG_CS_MEMBER: u8 = 0x31;

/// `envelope_version` written and accepted (§1.1).
pub const ENVELOPE_VERSION: u8 = 1;
/// `layer_version` written and accepted (UNIT-15, UNIT-16).
pub const LAYER_VERSION: u32 = 1;
/// The `LAYER_META` name of the layer mark (design §2.6).
pub const LAYER_MARK_NAME: &str = "layer";
/// Length limits of §1.2.
pub const MAX_LAYER_NAME_LEN: usize = 64;
pub const MAX_UNIT_TYPE_LEN: u32 = 64;
pub const MAX_METADATA_LEN: u32 = 4096;
pub const MAX_REV_REF_LEN: u32 = 256;
pub const MAX_CHANGESET_FIELD_LEN: u32 = 1024;
pub const MAX_MEMBER_COUNT: u32 = 1 << 20;

type Res<T> = Result<T, DecodeError>;

fn reject<T>(rule: UnitRule, detail: impl Into<String>) -> Res<T> {
    Err(DecodeError::new(rule, detail))
}

// ---------------------------------------------------------------- keys

/// A decoded key (`units-v1.md` §2). The derived order is the order of the
/// encoded bytes: variants follow their tags and fields follow byte order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    LayerMeta { name: String },
    UnitHead { unit: UnitId },
    RevMeta { unit: UnitId, rev: RevNo },
    RevPayload { unit: UnitId, rev: RevNo },
    ChangeSet { seq: CommitSeq },
    CsMember { seq: CommitSeq, ordinal: u32 },
}

impl Key {
    pub fn tag(&self) -> u8 {
        match self {
            Key::LayerMeta { .. } => TAG_LAYER_META,
            Key::UnitHead { .. } => TAG_UNIT_HEAD,
            Key::RevMeta { .. } => TAG_REV_META,
            Key::RevPayload { .. } => TAG_REV_PAYLOAD,
            Key::ChangeSet { .. } => TAG_CHANGESET,
            Key::CsMember { .. } => TAG_CS_MEMBER,
        }
    }

    /// The key bytes. Refuses a key the decoder would reject (an empty or
    /// over-long `LAYER_META` name, `rev_no = 0`, `commit_seq = 0`).
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let mut out = vec![self.tag()];
        match self {
            Key::LayerMeta { name } => out.extend_from_slice(name.as_bytes()),
            Key::UnitHead { unit } => out.extend_from_slice(&unit.0),
            Key::RevMeta { unit, rev } | Key::RevPayload { unit, rev } => {
                out.extend_from_slice(&unit.0);
                out.extend_from_slice(&rev.0.to_be_bytes());
            }
            Key::ChangeSet { seq } => out.extend_from_slice(&seq.0.to_be_bytes()),
            Key::CsMember { seq, ordinal } => {
                out.extend_from_slice(&seq.0.to_be_bytes());
                out.extend_from_slice(&ordinal.to_be_bytes());
            }
        }
        Key::decode(&out).map_err(|e| refuse("key", &e))?;
        Ok(out)
    }

    /// Decodes a key, applying UNIT-1..=UNIT-5 in that order.
    pub fn decode(key: &[u8]) -> Res<Key> {
        let Some((&tag, tail)) = key.split_first() else {
            return reject(UnitRule::Unit1, "empty key");
        };
        let want = match tag {
            TAG_LAYER_META => None,
            TAG_UNIT_HEAD => Some(17),
            TAG_REV_META | TAG_REV_PAYLOAD => Some(25),
            TAG_CHANGESET => Some(9),
            TAG_CS_MEMBER => Some(13),
            0x00..=0x3F => return reject(UnitRule::Unit1, format!("unknown tag 0x{tag:02x} in a known range")),
            _ => return reject(UnitRule::Unit2, format!("tag 0x{tag:02x} belongs to a newer layer")),
        };
        let len_ok = match want {
            Some(n) => key.len() == n,
            None => (2..=MAX_LAYER_NAME_LEN + 1).contains(&key.len()),
        };
        if !len_ok {
            return reject(UnitRule::Unit3, format!("key with tag 0x{tag:02x} has length {}", key.len()));
        }
        // The lengths are now fixed, so the slicing below is in bounds.
        let unit = || UnitId(arr::<16>(&tail[..16]));
        let u64_at = |at: usize| u64::from_be_bytes(arr::<8>(&tail[at..at + 8]));
        match tag {
            TAG_LAYER_META => match std::str::from_utf8(tail) {
                Ok(name) => Ok(Key::LayerMeta { name: name.to_owned() }),
                Err(_) => reject(UnitRule::Unit4, "LAYER_META name is not UTF-8"),
            },
            TAG_UNIT_HEAD => Ok(Key::UnitHead { unit: unit() }),
            TAG_REV_META | TAG_REV_PAYLOAD => {
                let rev = u64_at(16);
                if rev == 0 {
                    return reject(UnitRule::Unit5, "rev_no = 0 in the key");
                }
                let (unit, rev) = (unit(), RevNo(rev));
                Ok(if tag == TAG_REV_META { Key::RevMeta { unit, rev } } else { Key::RevPayload { unit, rev } })
            }
            _ => {
                let seq = u64_at(0);
                if seq == 0 {
                    return reject(UnitRule::Unit5, "commit_seq = 0 in the key");
                }
                Ok(if tag == TAG_CHANGESET {
                    Key::ChangeSet { seq: CommitSeq(seq) }
                } else {
                    Key::CsMember { seq: CommitSeq(seq), ordinal: u32::from_be_bytes(arr::<4>(&tail[8..12])) }
                })
            }
        }
    }

    /// Scan bounds `[start, end)` of every key with `tag`.
    pub fn tag_range(tag: u8) -> (Vec<u8>, Vec<u8>) {
        (vec![tag], vec![tag.saturating_add(1)])
    }

    /// Scan bounds `[start, end)` of every `REV_META` (or `REV_PAYLOAD`) of `unit`.
    pub fn unit_rev_range(tag: u8, unit: UnitId) -> (Vec<u8>, Vec<u8>) {
        let mut start = vec![tag];
        start.extend_from_slice(&unit.0);
        let mut end = start.clone();
        // Increment the 17-byte prefix as a big-endian number; tag < 0xFF so it
        // never overflows past the tag.
        for b in end.iter_mut().rev() {
            let (v, carry) = b.overflowing_add(1);
            *b = v;
            if !carry {
                break;
            }
        }
        (start, end)
    }
}

/// Copies an exactly-`N`-byte slice into an array. Callers pass slices whose
/// length was already checked; a mismatch yields zeroes instead of panicking.
fn arr<const N: usize>(s: &[u8]) -> [u8; N] {
    let mut a = [0u8; N];
    if s.len() == N {
        a.copy_from_slice(s);
    }
    a
}

fn refuse(what: &str, e: &DecodeError) -> UnitError {
    UnitError::InvalidArgument(format!("refusing to encode an invalid {what}: {e}"))
}

// ---------------------------------------------------------------- reader

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    rec: &'static str,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], rec: &'static str) -> Self {
        Reader { buf, pos: 0, rec }
    }

    fn take(&mut self, n: usize, field: &str) -> Res<&'a [u8]> {
        let left = self.buf.len() - self.pos;
        if n > left {
            return reject(
                UnitRule::Unit9,
                format!("{}.{field} needs {n} bytes at offset {}, {left} left", self.rec, self.pos),
            );
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self, field: &str) -> Res<u8> {
        Ok(self.take(1, field)?[0])
    }

    fn u32(&mut self, field: &str) -> Res<u32> {
        Ok(u32::from_be_bytes(arr(self.take(4, field)?)))
    }

    fn u64(&mut self, field: &str) -> Res<u64> {
        Ok(u64::from_be_bytes(arr(self.take(8, field)?)))
    }

    fn i64(&mut self, field: &str) -> Res<i64> {
        Ok(i64::from_be_bytes(arr(self.take(8, field)?)))
    }

    /// `bytes(max)`: UNIT-11 on the prefix before UNIT-9 on the body.
    fn bytes(&mut self, max: u32, field: &str) -> Res<&'a [u8]> {
        let len = self.u32(field)?;
        if len > max {
            return reject(UnitRule::Unit11, format!("{}.{field} length {len} exceeds {max}", self.rec));
        }
        // len ≤ max ≤ 4096, so the cast is exact on every target.
        self.take(len as usize, field)
    }

    fn envelope(&mut self) -> Res<()> {
        match self.u8("envelope_version")? {
            0 => return reject(UnitRule::Unit6, format!("{} envelope_version 0", self.rec)),
            ENVELOPE_VERSION => {}
            v => return reject(UnitRule::Unit7, format!("{} envelope_version {v} is newer than 1", self.rec)),
        }
        match self.u8("reserved_flags")? {
            0 => Ok(()),
            f => reject(UnitRule::Unit8, format!("{} reserved_flags 0x{f:02x}", self.rec)),
        }
    }

    fn content_hash(&mut self) -> Res<ContentHash> {
        let a = self.u8("content_hash.alg")?;
        let Some(alg) = HashAlg::from_byte(a) else {
            return reject(UnitRule::Unit13, format!("{}.content_hash.alg {a} is unknown", self.rec));
        };
        let digest: [u8; 32] = arr(self.take(32, "content_hash.digest")?);
        if alg == HashAlg::Crc32cProvisional && digest[4..].iter().any(|b| *b != 0) {
            return reject(UnitRule::Unit14, format!("{}.content_hash alg 0 with non-zero padding", self.rec));
        }
        Ok(ContentHash { alg, digest })
    }

    fn payload_len(&mut self) -> Res<u32> {
        let n = self.u32("payload_len")?;
        if n > MAX_PAYLOAD_LEN {
            return reject(UnitRule::Unit22, format!("{}.payload_len {n} exceeds {MAX_PAYLOAD_LEN}", self.rec));
        }
        Ok(n)
    }

    fn payload_codec(&mut self) -> Res<()> {
        match self.u8("payload_codec")? {
            PAYLOAD_CODEC_OPAQUE => Ok(()),
            c => reject(UnitRule::Unit13, format!("{}.payload_codec {c} is not 0", self.rec)),
        }
    }

    fn enumeration<T>(&mut self, field: &str, from: fn(u8) -> Option<T>) -> Res<T> {
        let b = self.u8(field)?;
        match from(b) {
            Some(v) => Ok(v),
            None => reject(UnitRule::Unit13, format!("{}.{field} {b} is outside the table", self.rec)),
        }
    }

    fn finish(&self) -> Res<()> {
        let extra = self.buf.len() - self.pos;
        if extra == 0 { Ok(()) } else { reject(UnitRule::Unit10, format!("{} has {extra} trailing bytes", self.rec)) }
    }
}

// ---------------------------------------------------------------- writer

struct Writer(Vec<u8>);

impl Writer {
    fn envelope() -> Self {
        Writer(vec![ENVELOPE_VERSION, 0])
    }

    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }

    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }

    /// `bytes(max)`; refuses a field longer than `max` (UNIT-11).
    fn bytes(&mut self, b: &[u8], max: u32, field: &str) -> Result<(), UnitError> {
        match u32::try_from(b.len()) {
            Ok(n) if n <= max => {
                self.u32(n);
                self.0.extend_from_slice(b);
                Ok(())
            }
            _ => Err(UnitError::InvalidArgument(format!(
                "refusing to encode an invalid value: {} {field} length {} exceeds {max}",
                UnitRule::Unit11,
                b.len()
            ))),
        }
    }

    fn hash(&mut self, h: &ContentHash) {
        self.u8(h.alg.to_byte());
        self.0.extend_from_slice(&h.digest);
    }
}

// ---------------------------------------------------------------- values

/// `LAYER_META` value (§3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerMeta {
    pub layer_version: u32,
    pub history_policy: HistoryPolicy,
}

impl LayerMeta {
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let mut w = Writer::envelope();
        w.u32(self.layer_version);
        w.u8(self.history_policy.to_byte());
        Self::decode(&w.0).map_err(|e| refuse("LAYER_META", &e))?;
        Ok(w.0)
    }

    pub fn decode(value: &[u8]) -> Res<Self> {
        let mut r = Reader::new(value, "LAYER_META");
        r.envelope()?;
        let layer_version = r.u32("layer_version")?;
        match layer_version {
            0 => return reject(UnitRule::Unit15, "LAYER_META.layer_version 0"),
            LAYER_VERSION => {}
            v => {
                let mut e = DecodeError::new(UnitRule::Unit16, format!("LAYER_META.layer_version {v} is newer than 1"));
                e.found_layer_version = Some(v);
                return Err(e);
            }
        }
        let history_policy = r.enumeration("history_policy", HistoryPolicy::from_byte)?;
        r.finish()?;
        Ok(LayerMeta { layer_version, history_policy })
    }
}

/// `UNIT_HEAD` value (§3.2). `payload_codec` is always 0 and not stored here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitHead {
    pub unit_type: String,
    pub schema_version: u32,
    pub lifecycle: Lifecycle,
    pub head_rev: RevNo,
    pub oldest_rev: RevNo,
    pub created_seq: CommitSeq,
    pub updated_seq: CommitSeq,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub content_hash: ContentHash,
    pub payload_len: u32,
    pub metadata: Vec<u8>,
}

impl UnitHead {
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let mut w = Writer::envelope();
        w.bytes(self.unit_type.as_bytes(), MAX_UNIT_TYPE_LEN, "UNIT_HEAD.unit_type")?;
        w.u32(self.schema_version);
        w.u8(self.lifecycle.to_byte());
        w.u64(self.head_rev.0);
        w.u64(self.oldest_rev.0);
        w.u64(self.created_seq.0);
        w.u64(self.updated_seq.0);
        w.i64(self.created_at.0);
        w.i64(self.updated_at.0);
        w.hash(&self.content_hash);
        w.u32(self.payload_len);
        w.u8(PAYLOAD_CODEC_OPAQUE);
        w.bytes(&self.metadata, MAX_METADATA_LEN, "UNIT_HEAD.metadata")?;
        Self::decode(&w.0).map_err(|e| refuse("UNIT_HEAD", &e))?;
        Ok(w.0)
    }

    pub fn decode(value: &[u8]) -> Res<Self> {
        let mut r = Reader::new(value, "UNIT_HEAD");
        r.envelope()?;
        let unit_type = r.bytes(MAX_UNIT_TYPE_LEN, "unit_type")?;
        let unit_type = match std::str::from_utf8(unit_type) {
            Ok(s) if !s.is_empty() => s.to_owned(),
            Ok(_) => return reject(UnitRule::Unit12, "UNIT_HEAD.unit_type is empty"),
            Err(_) => return reject(UnitRule::Unit12, "UNIT_HEAD.unit_type is not UTF-8"),
        };
        let schema_version = r.u32("schema_version")?;
        let lifecycle = r.enumeration("lifecycle", Lifecycle::from_byte)?;
        let head_rev = r.u64("head_rev")?;
        let oldest_rev = r.u64("oldest_rev")?;
        let created_seq = r.u64("created_seq")?;
        let updated_seq = r.u64("updated_seq")?;
        let created_at = r.i64("created_at")?;
        let updated_at = r.i64("updated_at")?;
        let content_hash = r.content_hash()?;
        let payload_len = r.payload_len()?;
        r.payload_codec()?;
        let metadata = r.bytes(MAX_METADATA_LEN, "metadata")?.to_vec();
        r.finish()?;
        if oldest_rev < 1 || oldest_rev > head_rev {
            return reject(UnitRule::Unit17, format!("UNIT_HEAD oldest_rev {oldest_rev}, head_rev {head_rev}"));
        }
        if created_seq < 1 || created_seq > updated_seq {
            return reject(UnitRule::Unit18, format!("UNIT_HEAD created_seq {created_seq}, updated_seq {updated_seq}"));
        }
        Ok(UnitHead {
            unit_type,
            schema_version,
            lifecycle,
            head_rev: RevNo(head_rev),
            oldest_rev: RevNo(oldest_rev),
            created_seq: CommitSeq(created_seq),
            updated_seq: CommitSeq(updated_seq),
            created_at: Timestamp(created_at),
            updated_at: Timestamp(updated_at),
            content_hash,
            payload_len,
            metadata,
        })
    }
}

/// `REV_META` value (§3.3). `parent_rev = 0` and `restored_from = 0` are
/// `None`; `payload_codec` is always 0 and not stored here. Its rules
/// between fields depend on the key's `rev_no`, so encode and decode take it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionMeta {
    pub kind: RevisionKind,
    pub parent_rev: Option<RevNo>,
    pub commit_seq: CommitSeq,
    pub created_at: Timestamp,
    pub schema_version: u32,
    pub content_hash: ContentHash,
    pub payload_len: u32,
    pub payload_present: bool,
    pub restored_from: Option<RevNo>,
    pub metadata: Vec<u8>,
    pub actor_ref: Vec<u8>,
    pub source_ref: Vec<u8>,
}

impl RevisionMeta {
    /// What this revision declares about its payload, for [`crate::verify_payload`].
    pub fn payload_decl(&self) -> PayloadDecl {
        PayloadDecl {
            content_hash: self.content_hash,
            payload_len: self.payload_len,
            payload_codec: PAYLOAD_CODEC_OPAQUE,
            payload_present: self.payload_present,
        }
    }

    pub fn encode(&self, rev_no: RevNo) -> Result<Vec<u8>, UnitError> {
        let mut w = Writer::envelope();
        w.u8(self.kind.to_byte());
        w.u64(self.parent_rev.map_or(0, |r| r.0));
        w.u64(self.commit_seq.0);
        w.i64(self.created_at.0);
        w.u32(self.schema_version);
        w.hash(&self.content_hash);
        w.u32(self.payload_len);
        w.u8(PAYLOAD_CODEC_OPAQUE);
        w.u8(u8::from(self.payload_present));
        w.u64(self.restored_from.map_or(0, |r| r.0));
        w.bytes(&self.metadata, MAX_METADATA_LEN, "REV_META.metadata")?;
        w.bytes(&self.actor_ref, MAX_REV_REF_LEN, "REV_META.actor_ref")?;
        w.bytes(&self.source_ref, MAX_REV_REF_LEN, "REV_META.source_ref")?;
        let back = Self::decode(rev_no, &w.0).map_err(|e| refuse("REV_META", &e))?;
        // `Some(RevNo(0))` encodes like `None`; refuse it instead of changing it.
        if back != *self {
            return Err(UnitError::InvalidArgument("refusing to encode an invalid REV_META: Some(0) revision".into()));
        }
        Ok(w.0)
    }

    #[inline]
    pub fn decode(rev_no: RevNo, value: &[u8]) -> Res<Self> {
        if let Some(m) = Self::decode_valid(rev_no, value) {
            return Ok(m);
        }
        Self::decode_checked(rev_no, value)
    }

    /// The fast path of [`decode`](Self::decode) (history pages decode one
    /// per revision): fixed offsets, no error values. It accepts only what
    /// [`decode_checked`](Self::decode_checked) accepts, with the same
    /// result, and answers `None` for anything else so that the checked
    /// decoder reports the exact rule.
    #[inline(always)]
    fn decode_valid(rev_no: RevNo, v: &[u8]) -> Option<Self> {
        const FIXED: usize = 78;
        let f: &[u8; FIXED] = v.get(..FIXED)?.try_into().ok()?;
        if f[0] != ENVELOPE_VERSION || f[1] != 0 {
            return None;
        }
        let u32_at = |at: usize| u32::from_be_bytes([f[at], f[at + 1], f[at + 2], f[at + 3]]);
        let u64_at = |at: usize| u64::from_be_bytes(arr(&f[at..at + 8]));
        let kind = RevisionKind::from_byte(f[2])?;
        let (parent_rev, commit_seq, created_at) = (u64_at(3), u64_at(11), u64_at(19) as i64);
        let schema_version = u32_at(27);
        let alg = HashAlg::from_byte(f[31])?;
        let digest: [u8; 32] = arr(&f[32..64]);
        let payload_len = u32_at(64);
        let (codec, present, restored_from) = (f[68], f[69], u64_at(70));
        if commit_seq == 0
            || (alg == HashAlg::Crc32cProvisional && digest[4..].iter().any(|b| *b != 0))
            || payload_len > MAX_PAYLOAD_LEN
            || codec != PAYLOAD_CODEC_OPAQUE
            || present > 1
        {
            return None;
        }
        fn field<'v>(v: &'v [u8], at: &mut usize, max: u32) -> Option<&'v [u8]> {
            let len = u32::from_be_bytes(arr(v.get(*at..*at + 4)?));
            if len > max {
                return None;
            }
            let body = v.get(*at + 4..*at + 4 + len as usize)?;
            *at += 4 + len as usize;
            Some(body)
        }
        // Empty fields (the common case) need no allocator call.
        let owned = |b: &[u8]| if b.is_empty() { Vec::new() } else { b.to_vec() };
        let mut at = FIXED;
        let metadata = owned(field(v, &mut at, MAX_METADATA_LEN)?);
        let actor_ref = owned(field(v, &mut at, MAX_REV_REF_LEN)?);
        let source_ref = owned(field(v, &mut at, MAX_REV_REF_LEN)?);
        if at != v.len() {
            return None;
        }
        let rev = rev_no.0;
        let parent_ok = if kind == RevisionKind::Create { parent_rev == 0 } else { (1..rev).contains(&parent_rev) };
        let restored_ok =
            if kind == RevisionKind::Restore { (1..rev).contains(&restored_from) } else { restored_from == 0 };
        let delete_ok = kind != RevisionKind::Delete || (present == 0 && payload_len == 0);
        if !(parent_ok && restored_ok && delete_ok) {
            return None;
        }
        Some(RevisionMeta {
            kind,
            parent_rev: (parent_rev != 0).then_some(RevNo(parent_rev)),
            commit_seq: CommitSeq(commit_seq),
            created_at: Timestamp(created_at),
            schema_version,
            content_hash: ContentHash { alg, digest },
            payload_len,
            payload_present: present == 1,
            restored_from: (restored_from != 0).then_some(RevNo(restored_from)),
            metadata,
            actor_ref,
            source_ref,
        })
    }

    /// The field-by-field decoder, which names the rule a value breaks.
    fn decode_checked(rev_no: RevNo, value: &[u8]) -> Res<Self> {
        let mut r = Reader::new(value, "REV_META");
        r.envelope()?;
        let kind = r.enumeration("kind", RevisionKind::from_byte)?;
        let parent_rev = r.u64("parent_rev")?;
        let commit_seq = r.u64("commit_seq")?;
        if commit_seq == 0 {
            return reject(UnitRule::Unit23, "REV_META.commit_seq 0");
        }
        let created_at = r.i64("created_at")?;
        let schema_version = r.u32("schema_version")?;
        let content_hash = r.content_hash()?;
        let payload_len = r.payload_len()?;
        r.payload_codec()?;
        let payload_present = r.u8("payload_present")?;
        let restored_from = r.u64("restored_from")?;
        let metadata = r.bytes(MAX_METADATA_LEN, "metadata")?.to_vec();
        let actor_ref = r.bytes(MAX_REV_REF_LEN, "actor_ref")?.to_vec();
        let source_ref = r.bytes(MAX_REV_REF_LEN, "source_ref")?.to_vec();
        r.finish()?;
        let rev = rev_no.0;
        let parent_ok = if kind == RevisionKind::Create { parent_rev == 0 } else { (1..rev).contains(&parent_rev) };
        if !parent_ok {
            return reject(UnitRule::Unit19, format!("REV_META {kind:?} rev_no {rev} with parent_rev {parent_rev}"));
        }
        let restored_ok =
            if kind == RevisionKind::Restore { (1..rev).contains(&restored_from) } else { restored_from == 0 };
        if !restored_ok {
            return reject(
                UnitRule::Unit20,
                format!("REV_META {kind:?} rev_no {rev} with restored_from {restored_from}"),
            );
        }
        if payload_present > 1 {
            return reject(UnitRule::Unit21, format!("REV_META.payload_present {payload_present}"));
        }
        if kind == RevisionKind::Delete && (payload_present != 0 || payload_len != 0) {
            return reject(
                UnitRule::Unit21,
                format!("REV_META Delete with payload_len {payload_len}, present {payload_present}"),
            );
        }
        Ok(RevisionMeta {
            kind,
            parent_rev: (parent_rev != 0).then_some(RevNo(parent_rev)),
            commit_seq: CommitSeq(commit_seq),
            created_at: Timestamp(created_at),
            schema_version,
            content_hash,
            payload_len,
            payload_present: payload_present == 1,
            restored_from: (restored_from != 0).then_some(RevNo(restored_from)),
            metadata,
            actor_ref,
            source_ref,
        })
    }
}

/// `CHANGESET` value (§3.5). Empty optional fields mean absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSetRecord {
    pub created_at: Timestamp,
    pub member_count: u32,
    pub purpose: Purpose,
    pub reason: Vec<u8>,
    pub actor_ref: Vec<u8>,
    pub source_ref: Vec<u8>,
    pub correlation: Vec<u8>,
}

impl ChangeSetRecord {
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let mut w = Writer::envelope();
        w.i64(self.created_at.0);
        w.u32(self.member_count);
        w.u8(self.purpose.to_byte());
        w.bytes(&self.reason, MAX_CHANGESET_FIELD_LEN, "CHANGESET.reason")?;
        w.bytes(&self.actor_ref, MAX_CHANGESET_FIELD_LEN, "CHANGESET.actor_ref")?;
        w.bytes(&self.source_ref, MAX_CHANGESET_FIELD_LEN, "CHANGESET.source_ref")?;
        w.bytes(&self.correlation, MAX_CHANGESET_FIELD_LEN, "CHANGESET.correlation")?;
        Self::decode(&w.0).map_err(|e| refuse("CHANGESET", &e))?;
        Ok(w.0)
    }

    pub fn decode(value: &[u8]) -> Res<Self> {
        let mut r = Reader::new(value, "CHANGESET");
        r.envelope()?;
        let created_at = r.i64("created_at")?;
        let member_count = r.u32("member_count")?;
        if member_count > MAX_MEMBER_COUNT {
            return reject(
                UnitRule::Unit24,
                format!("CHANGESET.member_count {member_count} exceeds {MAX_MEMBER_COUNT}"),
            );
        }
        let purpose = r.enumeration("purpose", Purpose::from_byte)?;
        let reason = r.bytes(MAX_CHANGESET_FIELD_LEN, "reason")?.to_vec();
        let actor_ref = r.bytes(MAX_CHANGESET_FIELD_LEN, "actor_ref")?.to_vec();
        let source_ref = r.bytes(MAX_CHANGESET_FIELD_LEN, "source_ref")?.to_vec();
        let correlation = r.bytes(MAX_CHANGESET_FIELD_LEN, "correlation")?.to_vec();
        r.finish()?;
        Ok(ChangeSetRecord {
            created_at: Timestamp(created_at),
            member_count,
            purpose,
            reason,
            actor_ref,
            source_ref,
            correlation,
        })
    }
}

/// `CS_MEMBER` value (§3.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsMember {
    pub unit: UnitId,
    pub rev_no: RevNo,
    pub kind: RevisionKind,
}

impl CsMember {
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let mut w = Writer::envelope();
        w.0.extend_from_slice(&self.unit.0);
        w.u64(self.rev_no.0);
        w.u8(self.kind.to_byte());
        Self::decode(&w.0).map_err(|e| refuse("CS_MEMBER", &e))?;
        Ok(w.0)
    }

    pub fn decode(value: &[u8]) -> Res<Self> {
        let mut r = Reader::new(value, "CS_MEMBER");
        r.envelope()?;
        let unit = UnitId(arr(r.take(16, "unit_id")?));
        let rev_no = r.u64("rev_no")?;
        if rev_no == 0 {
            return reject(UnitRule::Unit25, "CS_MEMBER.rev_no 0");
        }
        let kind = r.enumeration("kind", RevisionKind::from_byte)?;
        r.finish()?;
        Ok(CsMember { unit, rev_no: RevNo(rev_no), kind })
    }
}

// ---------------------------------------------------------------- records

/// A decoded `(key, value)` pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    LayerMeta {
        name: String,
        meta: LayerMeta,
    },
    UnitHead {
        unit: UnitId,
        head: UnitHead,
    },
    RevMeta {
        unit: UnitId,
        rev: RevNo,
        meta: RevisionMeta,
    },
    /// The payload as stored; its length and hash are checked against the
    /// matching `REV_META` by [`crate::verify_payload`], not here (§3.4).
    RevPayload {
        unit: UnitId,
        rev: RevNo,
        payload: Vec<u8>,
    },
    ChangeSet {
        seq: CommitSeq,
        record: ChangeSetRecord,
    },
    CsMember {
        seq: CommitSeq,
        ordinal: u32,
        member: CsMember,
    },
}

impl Record {
    pub fn key(&self) -> Key {
        match self {
            Record::LayerMeta { name, .. } => Key::LayerMeta { name: name.clone() },
            Record::UnitHead { unit, .. } => Key::UnitHead { unit: *unit },
            Record::RevMeta { unit, rev, .. } => Key::RevMeta { unit: *unit, rev: *rev },
            Record::RevPayload { unit, rev, .. } => Key::RevPayload { unit: *unit, rev: *rev },
            Record::ChangeSet { seq, .. } => Key::ChangeSet { seq: *seq },
            Record::CsMember { seq, ordinal, .. } => Key::CsMember { seq: *seq, ordinal: *ordinal },
        }
    }

    /// The `(key, value)` bytes; refuses a record the decoder would reject.
    pub fn encode(&self) -> Result<(Vec<u8>, Vec<u8>), UnitError> {
        let key = self.key().encode()?;
        let value = match self {
            Record::LayerMeta { meta, .. } => meta.encode()?,
            Record::UnitHead { head, .. } => head.encode()?,
            Record::RevMeta { rev, meta, .. } => meta.encode(*rev)?,
            Record::RevPayload { payload, .. } => payload.clone(),
            Record::ChangeSet { record, .. } => record.encode()?,
            Record::CsMember { member, .. } => member.encode()?,
        };
        Ok((key, value))
    }

    /// Decodes a pair in the order of `units-v1.md` §6 and returns the first
    /// rule broken.
    pub fn decode(key: &[u8], value: &[u8]) -> Res<Self> {
        Ok(match Key::decode(key)? {
            Key::LayerMeta { name } => Record::LayerMeta { name, meta: LayerMeta::decode(value)? },
            Key::UnitHead { unit } => Record::UnitHead { unit, head: UnitHead::decode(value)? },
            Key::RevMeta { unit, rev } => Record::RevMeta { unit, rev, meta: RevisionMeta::decode(rev, value)? },
            Key::RevPayload { unit, rev } => Record::RevPayload { unit, rev, payload: value.to_vec() },
            Key::ChangeSet { seq } => Record::ChangeSet { seq, record: ChangeSetRecord::decode(value)? },
            Key::CsMember { seq, ordinal } => Record::CsMember { seq, ordinal, member: CsMember::decode(value)? },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fast `REV_META` decoder accepts a subset of what the checked one
    /// accepts, with the same result, on valid values and on mutations of
    /// them (bytes changed, cut, extended).
    #[test]
    fn rev_meta_fast_path_agrees_with_checked() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let kinds = RevisionKind::ALL;
        let mut accepted = 0;
        for i in 0..20_000u64 {
            let kind = kinds[(next() % kinds.len() as u64) as usize];
            let rev = RevNo(if kind == RevisionKind::Create { 1 } else { 2 + next() % 5 });
            let alg = if next() % 2 == 0 { HashAlg::Blake3 } else { HashAlg::Crc32cProvisional };
            let mut digest = [0u8; 32];
            digest[..4].copy_from_slice(&(next() as u32).to_be_bytes());
            if alg == HashAlg::Blake3 {
                digest.iter_mut().for_each(|b| *b = next() as u8);
            }
            let deleted = kind == RevisionKind::Delete;
            let meta = RevisionMeta {
                kind,
                parent_rev: (kind != RevisionKind::Create).then_some(RevNo(rev.0 - 1)),
                commit_seq: CommitSeq(1 + next() % 1000),
                created_at: Timestamp(next() as i64),
                schema_version: next() as u32,
                content_hash: ContentHash { alg, digest },
                payload_len: if deleted { 0 } else { (next() % 5000) as u32 },
                payload_present: !deleted && next() % 2 == 0,
                restored_from: (kind == RevisionKind::Restore).then_some(RevNo(1)),
                metadata: vec![7; (next() % 3) as usize * 5],
                actor_ref: vec![8; (next() % 2) as usize * 3],
                source_ref: vec![9; (next() % 2) as usize],
            };
            let mut v = meta.encode(rev).expect("encode");
            match i % 4 {
                0 => {}
                1 => {
                    let at = (next() % v.len() as u64) as usize;
                    v[at] = next() as u8;
                }
                2 => v.truncate((next() % v.len() as u64) as usize),
                _ => v.push(next() as u8),
            }
            let fast = RevisionMeta::decode_valid(rev, &v);
            let checked = RevisionMeta::decode_checked(rev, &v);
            if let Some(m) = &fast {
                accepted += 1;
                assert_eq!(Some(m), checked.as_ref().ok(), "value {v:?}");
            }
            if i % 4 == 0 {
                assert_eq!(fast, Some(meta), "a valid value takes the fast path");
            }
        }
        assert!(accepted > 5_000);
    }
}
