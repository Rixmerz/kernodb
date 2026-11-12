//! E8-04: persistence of vectors as derived data (ADR-0020, layer version 2,
//! `docs/file-format/units-v2.md`).
//!
//! A vector is a derived datum. It is stored in the units key space, in three
//! records per datum:
//!
//! | tag | record | key after the tag |
//! |---|---|---|
//! | `0x60` | descriptor | `derived_id[32]` |
//! | `0x61` | vector | `derived_id[32]` |
//! | `0x62` | inverse index | `unit_id[16] ‖ rev_no ‖ derived_id[32]` |
//!
//! # Contracts
//!
//! * **Never a source of truth.** The unit is. A vector that is lost, damaged
//!   or no longer matches its source is thrown away and recomputed
//!   ([`DerivedDb::rebuild_vector`]); the result equals a fresh build (I-24).
//! * **One transaction (I-21).** The descriptor, the vector, the `0x62` entry
//!   and, for the first one, the layer mark going to version 2 are written by
//!   one change set. A change to the source marks the derived data stale, and
//!   a deletion removes them, in the transaction of that change: the plain
//!   [`crate::ChangeSet`] does it, so no path commits a source change without
//!   its dependents (I-23, I-25, I-38).
//! * **Reject, never misread.** A build that only knows layer version 1 reads
//!   the mark first and refuses (UNIT-16). The version goes to 2 only with
//!   the first persisted vector and is never lowered.
//! * **Opening writes nothing.** [`DerivedDb::open_persisted`] rebuilds the
//!   registry from the descriptors, checks every `Ready` vector against its
//!   output hash and serves nothing it could not check. Repair is the explicit
//!   [`DerivedDb::repair_derived`].
//! * **No content in errors (I-37).** Errors and `Debug` output name rules,
//!   record types, ids and numbers; never a vector element.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Bound;

use kerno_core::contract::{Snapshot, Txn, TxnEngine};

use crate::codec::{
    ENVELOPE_VERSION, Key, LAYER_VERSION_DERIVED, TAG_DERIVED_DESC, TAG_DERIVED_SRC, TAG_DERIVED_VEC, UnitHead,
};
use crate::db::{ChangeMeta, ChangeSet, CommitInfo, Write};
use crate::derived::{DerivedDescriptor, DerivedError, DerivedId, DerivedState};
use crate::derived_stale::{DerivedChangeSet, DerivedDb, DerivedStaleError, Pending, SourceInput, input_hash_of_ids};
use crate::error::{ErrorClass, UnitError, UnitRule};
use crate::types::{Lifecycle, RevNo, RevisionId, UnitId};

/// Largest vector dimension (DV-6).
pub const MAX_DIM: u32 = 4096;
/// `elem_type` of IEEE 754 binary32 (DV-7).
pub const ELEM_F32: u8 = 1;
/// `kind` of a vector descriptor (DV-1).
pub const KIND_VECTOR: u8 = 1;
/// Limits of the descriptor text fields (DV-4, DV-5).
pub const MAX_METHOD_LEN: usize = 64;
pub const MAX_PROFILE_LEN: usize = 64;
pub const MAX_PARAMS: usize = 16;
pub const MAX_PARAM_KEY_LEN: usize = 64;
pub const MAX_PARAM_VALUE_LEN: usize = 256;

// ------------------------------------------------------------------ rules

/// A rule of ADR-0020 §D that a record can break: `DV-n` of the persisted
/// derived data, or a `UNIT-n` rule of layer 1 that applies to the new values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DerivedRule {
    /// `DV-n`, `n` in `1..=14`.
    Dv(u8),
    Unit(UnitRule),
}

impl DerivedRule {
    /// `"DV-n"` or `"UNIT-n"`.
    pub fn id(self) -> String {
        match self {
            DerivedRule::Dv(n) => format!("DV-{n}"),
            DerivedRule::Unit(r) => r.id(),
        }
    }

    /// Every `DV-n` is `Corruption`; a `UNIT-n` keeps its class.
    pub fn class(self) -> ErrorClass {
        match self {
            DerivedRule::Dv(_) => ErrorClass::Corruption,
            DerivedRule::Unit(r) => r.class(),
        }
    }
}

impl fmt::Display for DerivedRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id())
    }
}

const DV1: DerivedRule = DerivedRule::Dv(1);
const DV2: DerivedRule = DerivedRule::Dv(2);
const DV3: DerivedRule = DerivedRule::Dv(3);
const DV4: DerivedRule = DerivedRule::Dv(4);
const DV5: DerivedRule = DerivedRule::Dv(5);
const DV6: DerivedRule = DerivedRule::Dv(6);
const DV7: DerivedRule = DerivedRule::Dv(7);
const DV8: DerivedRule = DerivedRule::Dv(8);
const DV9: DerivedRule = DerivedRule::Dv(9);
const DV10: DerivedRule = DerivedRule::Dv(10);
const DV11: DerivedRule = DerivedRule::Dv(11);
const DV12: DerivedRule = DerivedRule::Dv(12);
const DV14: DerivedRule = DerivedRule::Dv(14);

/// A key or value that breaks a rule: the first one, with a detail that names
/// fields and numbers only (I-37).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedDecodeError {
    pub rule: DerivedRule,
    pub detail: String,
}

impl DerivedDecodeError {
    fn new(rule: DerivedRule, detail: impl Into<String>) -> Self {
        DerivedDecodeError { rule, detail: detail.into() }
    }

    pub fn class(&self) -> ErrorClass {
        self.rule.class()
    }
}

impl fmt::Display for DerivedDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.rule, self.class(), self.detail)
    }
}

impl std::error::Error for DerivedDecodeError {}

impl From<DerivedDecodeError> for UnitError {
    fn from(e: DerivedDecodeError) -> Self {
        let msg = e.to_string();
        UnitError::Storage(match e.class() {
            ErrorClass::Corruption => kerno_core::Error::Corruption(msg),
            ErrorClass::IncompatibleLayer => kerno_core::Error::Incompatible(msg),
        })
    }
}

type Res<T> = Result<T, DerivedDecodeError>;

fn reject<T>(rule: DerivedRule, detail: impl Into<String>) -> Res<T> {
    Err(DerivedDecodeError::new(rule, detail))
}

fn unit_rule<T>(rule: UnitRule, detail: impl Into<String>) -> Res<T> {
    reject(DerivedRule::Unit(rule), detail)
}

// ------------------------------------------------------------------ keys

/// A key of the persisted derived data (ADR-0020 §A).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DerivedKey {
    Desc(DerivedId),
    Vec(DerivedId),
    Src { unit: UnitId, rev: RevNo, id: DerivedId },
}

fn arr<const N: usize>(s: &[u8]) -> [u8; N] {
    let mut a = [0u8; N];
    if s.len() == N {
        a.copy_from_slice(s);
    }
    a
}

impl DerivedKey {
    pub fn tag(&self) -> u8 {
        match self {
            DerivedKey::Desc(_) => TAG_DERIVED_DESC,
            DerivedKey::Vec(_) => TAG_DERIVED_VEC,
            DerivedKey::Src { .. } => TAG_DERIVED_SRC,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.tag()];
        match self {
            DerivedKey::Desc(id) | DerivedKey::Vec(id) => out.extend_from_slice(&id.0),
            DerivedKey::Src { unit, rev, id } => {
                out.extend_from_slice(&unit.0);
                out.extend_from_slice(&rev.0.to_be_bytes());
                out.extend_from_slice(&id.0);
            }
        }
        out
    }

    /// Decodes a key of tag `0x60..=0x62`. DV-3 for a wrong length, UNIT-5 for
    /// `rev_no = 0`, UNIT-2 for any other tag (a newer layer's, or the
    /// reserved `0x63..=0x6F`).
    pub fn decode(key: &[u8]) -> Res<DerivedKey> {
        let Some((&tag, tail)) = key.split_first() else {
            return unit_rule(UnitRule::Unit1, "empty key");
        };
        let want = match tag {
            TAG_DERIVED_DESC | TAG_DERIVED_VEC => 33,
            TAG_DERIVED_SRC => 57,
            _ => return unit_rule(UnitRule::Unit2, format!("tag 0x{tag:02x} belongs to a newer layer")),
        };
        if key.len() != want {
            return reject(DV3, format!("key with tag 0x{tag:02x} has length {}", key.len()));
        }
        Ok(match tag {
            TAG_DERIVED_DESC => DerivedKey::Desc(DerivedId(arr(tail))),
            TAG_DERIVED_VEC => DerivedKey::Vec(DerivedId(arr(tail))),
            _ => {
                let rev = u64::from_be_bytes(arr(&tail[16..24]));
                if rev == 0 {
                    return unit_rule(UnitRule::Unit5, "rev_no = 0 in the key");
                }
                DerivedKey::Src { unit: UnitId(arr(&tail[..16])), rev: RevNo(rev), id: DerivedId(arr(&tail[24..])) }
            }
        })
    }

    /// `[start, end)` of every key with `tag`.
    pub fn tag_range(tag: u8) -> (Vec<u8>, Vec<u8>) {
        Key::tag_range(tag)
    }
}

fn desc_key(id: DerivedId) -> Vec<u8> {
    DerivedKey::Desc(id).encode()
}

fn vec_key(id: DerivedId) -> Vec<u8> {
    DerivedKey::Vec(id).encode()
}

fn src_key(source: RevisionId, id: DerivedId) -> Vec<u8> {
    DerivedKey::Src { unit: source.unit, rev: source.rev, id }.encode()
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
            return unit_rule(
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

    fn envelope(&mut self) -> Res<()> {
        match self.u8("envelope_version")? {
            0 => return unit_rule(UnitRule::Unit6, format!("{} envelope_version 0", self.rec)),
            ENVELOPE_VERSION => {}
            v => return unit_rule(UnitRule::Unit7, format!("{} envelope_version {v} is newer than 1", self.rec)),
        }
        match self.u8("reserved_flags")? {
            0 => Ok(()),
            f => unit_rule(UnitRule::Unit8, format!("{} reserved_flags 0x{f:02x}", self.rec)),
        }
    }

    /// `str(max)` with `rule` for an empty (when `!empty_ok`), over-long or
    /// non-UTF-8 text. The length is compared with `max` before the body is
    /// touched, so no allocation follows an unchecked length.
    fn text(&mut self, max: usize, empty_ok: bool, rule: DerivedRule, field: &str) -> Res<String> {
        let len = self.u32(field)? as usize;
        if len > max || (len == 0 && !empty_ok) {
            return reject(
                rule,
                format!("{}.{field} length {len} is outside {}..={max}", self.rec, usize::from(!empty_ok)),
            );
        }
        match std::str::from_utf8(self.take(len, field)?) {
            Ok(s) => Ok(s.to_owned()),
            Err(_) => reject(rule, format!("{}.{field} is not UTF-8", self.rec)),
        }
    }

    fn finish(&self) -> Res<()> {
        let extra = self.buf.len() - self.pos;
        if extra == 0 {
            Ok(())
        } else {
            unit_rule(UnitRule::Unit10, format!("{} has {extra} trailing bytes", self.rec))
        }
    }
}

// ------------------------------------------------------------ descriptor

/// The persisted state of a derived datum (`state` byte of ADR-0020 §B.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VectorState {
    Ready,
    Stale,
    Superseded,
}

impl VectorState {
    pub fn to_byte(self) -> u8 {
        match self {
            VectorState::Ready => 1,
            VectorState::Stale => 2,
            VectorState::Superseded => 3,
        }
    }

    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(VectorState::Ready),
            2 => Some(VectorState::Stale),
            3 => Some(VectorState::Superseded),
            _ => None,
        }
    }

    fn to_derived(self) -> DerivedState {
        match self {
            VectorState::Ready => DerivedState::Ready,
            VectorState::Stale => DerivedState::Stale,
            VectorState::Superseded => DerivedState::Superseded,
        }
    }
}

/// The `DERIVED_DESC` value (ADR-0020 §B.1).
#[derive(Clone, PartialEq, Eq)]
pub struct VectorDescriptor {
    pub state: VectorState,
    pub source: RevisionId,
    pub method: String,
    pub method_version: u32,
    pub profile: String,
    pub params: BTreeMap<String, String>,
    pub input_hash: [u8; 32],
    pub output_hash: [u8; 32],
}

impl fmt::Debug for VectorDescriptor {
    /// The text fields are caller-supplied: only their lengths are shown (I-37).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VectorDescriptor")
            .field("state", &self.state)
            .field("source", &self.source)
            .field("method_len", &self.method.len())
            .field("method_version", &self.method_version)
            .field("profile_len", &self.profile.len())
            .field("params", &self.params.len())
            .finish_non_exhaustive()
    }
}

impl VectorDescriptor {
    /// The registry's view of this descriptor.
    pub fn to_descriptor(&self) -> DerivedDescriptor {
        // `decode` and `encode` guarantee a non-empty method; a descriptor
        // built by hand with an empty one has no identity, hence the unwrap.
        DerivedDescriptor::new(
            [self.source],
            self.method.clone(),
            self.method_version,
            self.params.clone(),
            self.input_hash,
        )
        .map(|d| d.with_profile(self.profile.clone()))
        .unwrap_or_else(|_| unreachable_descriptor())
    }

    /// The identity of §C.1.
    pub fn id(&self) -> DerivedId {
        self.to_descriptor().id()
    }

    /// The value bytes. Refuses a descriptor the decoder would reject.
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let mut out = vec![ENVELOPE_VERSION, 0, KIND_VECTOR, self.state.to_byte()];
        out.extend_from_slice(&self.source.unit.0);
        out.extend_from_slice(&self.source.rev.0.to_be_bytes());
        put_text(&mut out, &self.method);
        out.extend_from_slice(&self.method_version.to_be_bytes());
        put_text(&mut out, &self.profile);
        out.push(u8::try_from(self.params.len()).unwrap_or(u8::MAX));
        for (k, v) in &self.params {
            put_text(&mut out, k);
            put_text(&mut out, v);
        }
        out.extend_from_slice(&self.input_hash);
        out.extend_from_slice(&self.output_hash);
        Self::decode(&out)
            .map_err(|e| UnitError::InvalidArgument(format!("refusing to encode an invalid descriptor: {e}")))?;
        Ok(out)
    }

    pub fn decode(value: &[u8]) -> Res<Self> {
        let mut r = Reader::new(value, "DERIVED_DESC");
        r.envelope()?;
        let kind = r.u8("kind")?;
        if kind != KIND_VECTOR {
            return reject(DV1, format!("DERIVED_DESC.kind {kind} is not 1"));
        }
        let state = r.u8("state")?;
        let Some(state) = VectorState::from_byte(state) else {
            return reject(DV2, format!("DERIVED_DESC.state {state} is outside the table"));
        };
        let unit = UnitId(arr(r.take(16, "source_unit")?));
        let rev = r.u64("source_rev")?;
        if rev == 0 {
            return reject(DV4, "DERIVED_DESC.source_rev is 0");
        }
        let method = r.text(MAX_METHOD_LEN, false, DV4, "method")?;
        let method_version = r.u32("method_version")?;
        let profile = r.text(MAX_PROFILE_LEN, false, DV4, "profile")?;
        let count = r.u8("param_count")? as usize;
        if count > MAX_PARAMS {
            return reject(DV5, format!("DERIVED_DESC.param_count {count} exceeds {MAX_PARAMS}"));
        }
        let mut params = BTreeMap::new();
        let mut last: Option<String> = None;
        for _ in 0..count {
            let k = r.text(MAX_PARAM_KEY_LEN, false, DV5, "param key")?;
            let v = r.text(MAX_PARAM_VALUE_LEN, true, DV5, "param value")?;
            if last.as_ref().is_some_and(|l| l.as_bytes() >= k.as_bytes()) {
                return reject(DV5, "DERIVED_DESC param keys are not strictly increasing");
            }
            last = Some(k.clone());
            params.insert(k, v);
        }
        let input_hash = arr(r.take(32, "input_hash")?);
        let output_hash = arr(r.take(32, "output_hash")?);
        r.finish()?;
        Ok(VectorDescriptor {
            state,
            source: RevisionId { unit, rev: RevNo(rev) },
            method,
            method_version,
            profile,
            params,
            input_hash,
            output_hash,
        })
    }
}

/// Unreachable for a descriptor that came from `decode`; kept as a function so
/// the caller of `to_descriptor` on a hand-built one does not panic.
fn unreachable_descriptor() -> DerivedDescriptor {
    DerivedDescriptor::new(
        [RevisionId { unit: UnitId([0; 16]), rev: RevNo(1) }],
        "invalid",
        0,
        BTreeMap::new(),
        [0; 32],
    )
    .unwrap_or_else(|_| unreachable!("a non-empty method and one source always make a descriptor"))
}

fn put_text(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

// ---------------------------------------------------------------- values

/// The `DERIVED_VEC` value of `data` (ADR-0020 §B.2): `-0.0` is written as
/// `+0.0` so the output hash does not depend on the sign of zero. A
/// dimension outside `1..=4096` or a non-finite element is refused.
pub fn encode_vector(data: &[f32]) -> Result<Vec<u8>, UnitError> {
    let dim = u32::try_from(data.len()).unwrap_or(u32::MAX);
    if dim == 0 || dim > MAX_DIM {
        return Err(UnitError::InvalidArgument(format!("{} (vector dimension {dim} is outside 1..={MAX_DIM})", DV6)));
    }
    if let Some(i) = data.iter().position(|x| !x.is_finite()) {
        return Err(UnitError::InvalidArgument(format!("{DV8} (element {i} of the vector is not finite)")));
    }
    let mut out = vec![ENVELOPE_VERSION, 0];
    out.extend_from_slice(&dim.to_be_bytes());
    out.push(ELEM_F32);
    for x in data {
        let x = if *x == 0.0 { 0.0 } else { *x };
        out.extend_from_slice(&x.to_bits().to_be_bytes());
    }
    Ok(out)
}

/// Decodes a `DERIVED_VEC` value: DV-6, DV-7 and DV-8 after the envelope.
pub fn decode_vector(value: &[u8]) -> Res<Vec<f32>> {
    let mut r = Reader::new(value, "DERIVED_VEC");
    r.envelope()?;
    let dim = r.u32("dim")?;
    if dim == 0 || dim > MAX_DIM {
        return reject(DV6, format!("DERIVED_VEC.dim {dim} is outside 1..={MAX_DIM}"));
    }
    let elem = r.u8("elem_type")?;
    if elem != ELEM_F32 {
        return reject(DV7, format!("DERIVED_VEC.elem_type {elem} is not 1"));
    }
    let want = 7 + 4 * dim as usize;
    if value.len() != want {
        return reject(DV6, format!("DERIVED_VEC has {} bytes, dim {dim} needs {want}", value.len()));
    }
    let (chunks, _) = value[7..].as_chunks::<4>();
    let mut out = Vec::with_capacity(chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        let bits = u32::from_be_bytes(*c);
        let x = f32::from_bits(bits);
        if !x.is_finite() || bits == 0x8000_0000 {
            return reject(DV8, format!("DERIVED_VEC element {i} is not finite or is -0.0"));
        }
        out.push(x);
    }
    Ok(out)
}

/// The output hash of §C.3: BLAKE3-256 of the whole `DERIVED_VEC` value.
pub fn output_hash(value: &[u8]) -> [u8; 32] {
    *blake3::hash(value).as_bytes()
}

/// The `DERIVED_SRC` value (§B.3).
pub fn encode_src_entry() -> Vec<u8> {
    vec![ENVELOPE_VERSION, 0]
}

pub fn decode_src_entry(value: &[u8]) -> Res<()> {
    let mut r = Reader::new(value, "DERIVED_SRC");
    r.envelope()?;
    r.finish()
}

// ---------------------------------------------------------------- reading

type Pair = (Vec<u8>, Vec<u8>);

/// What the planners and the checks read: a transaction or a snapshot.
pub(crate) trait Source {
    fn get(&self, key: &[u8]) -> kerno_core::Result<Option<Vec<u8>>>;
    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> kerno_core::Result<Vec<Pair>>;
}

pub(crate) struct TxnReader<'a, T: Txn>(pub(crate) &'a T);

impl<T: Txn> Source for TxnReader<'_, T> {
    fn get(&self, key: &[u8]) -> kerno_core::Result<Option<Vec<u8>>> {
        self.0.get(key)
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> kerno_core::Result<Vec<Pair>> {
        self.0.scan(start, end, limit)
    }
}

pub(crate) struct SnapReader<'a, S: Snapshot>(pub(crate) &'a S);

impl<S: Snapshot> Source for SnapReader<'_, S> {
    fn get(&self, key: &[u8]) -> kerno_core::Result<Option<Vec<u8>>> {
        self.0.get(key)
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> kerno_core::Result<Vec<Pair>> {
        self.0.scan(start, end, limit)
    }
}

const PAGE: usize = 256;

/// Every pair in `[start, end)`, in key order, `PAGE` per scan.
fn scan_pairs<R: Source>(r: &R, start: &[u8], end: &[u8]) -> Result<Vec<Pair>, UnitError> {
    let mut out = Vec::new();
    let mut last: Option<Vec<u8>> = None;
    loop {
        let lo = match &last {
            Some(k) => Bound::Excluded(k.as_slice()),
            None => Bound::Included(start),
        };
        let page = r.scan(lo, Bound::Excluded(end), PAGE)?;
        let n = page.len();
        last = page.last().map(|(k, _)| k.clone());
        out.extend(page);
        if n < PAGE {
            return Ok(out);
        }
    }
}

// ------------------------------------------------- in-transaction follow-up

/// What happened to a source, as the derived data of its unit must hear it.
pub(crate) enum SourceEvent {
    /// A revision other than `Delete` was written: `Ready` goes `Stale`.
    Changed,
    /// The unit was deleted: its derived data are removed.
    Deleted,
    /// Retention purged the revisions below this one: the derived data
    /// computed from them are removed.
    Purged(RevNo),
}

/// The writes that keep the persisted derived data of `unit` in step with a
/// source event (ADR-0020 §S3.2, §S3.3, §S5.1). Reads only.
pub(crate) fn plan_source_event<R: Source>(r: &R, unit: UnitId, event: SourceEvent) -> Result<Vec<Write>, UnitError> {
    let (start, mut end) = Key::unit_rev_range(TAG_DERIVED_SRC, unit);
    if let SourceEvent::Purged(below) = event {
        end = start.clone();
        end.extend_from_slice(&below.0.to_be_bytes());
    }
    let mut writes = Vec::new();
    for (k, _) in scan_pairs(r, &start, &end)? {
        let DerivedKey::Src { id, .. } = DerivedKey::decode(&k)? else {
            return Err(UnitError::from(DerivedDecodeError::new(DV3, "a key in a DERIVED_SRC range is not one")));
        };
        match event {
            SourceEvent::Deleted | SourceEvent::Purged(_) => {
                writes.push(Write::Delete(desc_key(id)));
                writes.push(Write::Delete(vec_key(id)));
                writes.push(Write::Delete(k));
            }
            SourceEvent::Changed => {
                let Some(raw) = r.get(&desc_key(id))? else { continue };
                // A descriptor that does not decode is never served (open
                // skips it, `verify_units` reports it): a source change must
                // not fail for derived data that cannot be read, so it is
                // left for the explicit repair.
                let Ok(mut d) = VectorDescriptor::decode(&raw) else { continue };
                if d.state == VectorState::Ready {
                    d.state = VectorState::Stale;
                    writes.push(Write::Put(desc_key(id), d.encode()?));
                    writes.push(Write::Delete(vec_key(id)));
                }
            }
        }
    }
    Ok(writes)
}

// -------------------------------------------------------------- analysis

/// A broken rule found in the persisted derived data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedFinding {
    pub rule: DerivedRule,
    pub key: Vec<u8>,
    /// Ids and numbers; never a vector element (I-37).
    pub detail: String,
}

/// Why a `Ready` descriptor is not served as `Ready`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleCause {
    /// The head of the source is another revision, or its input hash differs.
    SourceMoved,
    /// No vector record (DV-12).
    VectorMissing,
    /// The vector does not hash to the descriptor's output hash (DV-12).
    VectorMismatch,
    /// The vector record does not decode (DV-6, DV-7, DV-8, UNIT-n).
    VectorUndecodable,
}

pub(crate) struct Loaded {
    pub desc: VectorDescriptor,
    /// The verified vector, only for a descriptor served as `Ready`.
    pub vector: Option<Vec<f32>>,
    /// Set when the persisted state is `Ready` and it is not served as such.
    pub cause: Option<StaleCause>,
}

#[derive(Default)]
pub(crate) struct Analysis {
    pub findings: Vec<DerivedFinding>,
    pub entries: BTreeMap<DerivedId, Loaded>,
    /// The writes of the explicit repair.
    pub repair: Vec<Write>,
}

impl Analysis {
    fn find(&mut self, rule: DerivedRule, key: &[u8], detail: impl Into<String>) {
        self.findings.push(DerivedFinding { rule, key: key.to_vec(), detail: detail.into() });
    }
}

enum Stored {
    Good(Vec<f32>),
    Bad(StaleCause),
}

/// Reads every persisted derived record and decides what each descriptor is
/// worth (ADR-0020 "Reconstrucción y pérdida", the table of §S4). `mark` is
/// the `layer_version` of the mark, `None` without one. Reads only.
pub(crate) fn analyze<R: Source>(r: &R, mark: Option<u32>) -> Result<Analysis, UnitError> {
    let mut a = Analysis::default();
    let (lo, hi) = (vec![TAG_DERIVED_DESC], vec![TAG_DERIVED_SRC + 1]);
    if mark.unwrap_or(0) < LAYER_VERSION_DERIVED {
        if let Some((k, _)) = r.scan(Bound::Included(&lo), Bound::Excluded(&hi), 1)?.into_iter().next() {
            a.find(DV11, &k, "persisted derived records in a database whose layer mark is version 1");
        }
        return Ok(a);
    }
    let (vec_start, src_start) = (vec![TAG_DERIVED_VEC], vec![TAG_DERIVED_SRC]);
    let mut descs: BTreeMap<DerivedId, VectorDescriptor> = BTreeMap::new();
    let mut bad: BTreeSet<DerivedId> = BTreeSet::new();
    // 0x60: descriptors, each against its own key (DV-10).
    for (k, v) in scan_pairs(r, &lo, &vec_start)? {
        let id = match DerivedKey::decode(&k) {
            Ok(DerivedKey::Desc(id)) => id,
            other => {
                a.find(key_rule(other), &k, "a key in the DERIVED_DESC range does not decode");
                a.repair.push(Write::Delete(k));
                continue;
            }
        };
        match VectorDescriptor::decode(&v) {
            Err(e) => {
                a.find(e.rule, &k, format!("DERIVED_DESC {}: {}", hex(&id.0), e.detail));
                bad.insert(id);
                a.repair.push(Write::Delete(k));
            }
            Ok(d) if d.id() != id => {
                a.find(
                    DV10,
                    &k,
                    format!("DERIVED_DESC {}: the identity recomputed from its fields differs", hex(&id.0)),
                );
                bad.insert(id);
                a.repair.push(Write::Delete(k));
            }
            Ok(d) => {
                descs.insert(id, d);
            }
        }
    }
    // 0x61: vectors, each against its descriptor (DV-9, DV-12).
    let mut vectors: BTreeMap<DerivedId, Stored> = BTreeMap::new();
    for (k, v) in scan_pairs(r, &vec_start, &src_start)? {
        let id = match DerivedKey::decode(&k) {
            Ok(DerivedKey::Vec(id)) => id,
            other => {
                a.find(key_rule(other), &k, "a key in the DERIVED_VEC range does not decode");
                a.repair.push(Write::Delete(k));
                continue;
            }
        };
        if bad.contains(&id) {
            a.repair.push(Write::Delete(k));
            continue;
        }
        let Some(d) = descs.get(&id) else {
            a.find(DV9, &k, format!("DERIVED_VEC {} without its DERIVED_DESC", hex(&id.0)));
            a.repair.push(Write::Delete(k));
            continue;
        };
        if d.state != VectorState::Ready {
            a.find(DV12, &k, format!("DERIVED_VEC {} of a {:?} descriptor", hex(&id.0), d.state));
            a.repair.push(Write::Delete(k));
            continue;
        }
        match decode_vector(&v) {
            Err(e) => {
                a.find(e.rule, &k, format!("DERIVED_VEC {}: {}", hex(&id.0), e.detail));
                vectors.insert(id, Stored::Bad(StaleCause::VectorUndecodable));
            }
            Ok(_) if output_hash(&v) != d.output_hash => {
                a.find(DV12, &k, format!("DERIVED_VEC {}: the value does not hash to the output hash", hex(&id.0)));
                vectors.insert(id, Stored::Bad(StaleCause::VectorMismatch));
            }
            Ok(x) => {
                vectors.insert(id, Stored::Good(x));
            }
        }
    }
    // 0x62: the inverse index against the descriptors (DV-9).
    let mut seen: BTreeSet<DerivedId> = BTreeSet::new();
    for (k, v) in scan_pairs(r, &src_start, &hi)? {
        let (unit, rev, id) = match DerivedKey::decode(&k) {
            Ok(DerivedKey::Src { unit, rev, id }) => (unit, rev, id),
            other => {
                a.find(key_rule(other), &k, "a key in the DERIVED_SRC range does not decode");
                a.repair.push(Write::Delete(k));
                continue;
            }
        };
        if let Err(e) = decode_src_entry(&v) {
            a.find(e.rule, &k, format!("DERIVED_SRC {}: {}", hex(&id.0), e.detail));
            a.repair.push(Write::Delete(k));
            continue;
        }
        if bad.contains(&id) {
            a.repair.push(Write::Delete(k));
            continue;
        }
        let Some(d) = descs.get(&id) else {
            a.find(DV9, &k, format!("DERIVED_SRC {} without its DERIVED_DESC", hex(&id.0)));
            a.repair.push(Write::Delete(k));
            continue;
        };
        seen.insert(id);
        if d.source != (RevisionId { unit, rev }) {
            a.find(DV9, &k, format!("DERIVED_SRC {} names another source than its descriptor", hex(&id.0)));
            a.repair.push(Write::Delete(k));
            a.repair.push(Write::Put(src_key(d.source, id), encode_src_entry()));
        }
    }
    // Each descriptor: its entry, its source, its vector.
    for (id, d) in descs {
        if !seen.contains(&id) {
            a.find(DV9, &desc_key(id), format!("DERIVED_DESC {} without a DERIVED_SRC entry", hex(&id.0)));
            a.repair.push(Write::Put(src_key(d.source, id), encode_src_entry()));
        }
        let head_raw = r.get(&Key::UnitHead { unit: d.source.unit }.encode()?)?;
        let head = head_raw.as_deref().and_then(|raw| UnitHead::decode(raw).ok());
        // An unreadable head is the verifier's finding, not a missing source.
        let gone = match (&head_raw, &head) {
            (None, _) => true,
            (Some(_), Some(h)) => h.lifecycle == Lifecycle::Deleted,
            (Some(_), None) => false,
        };
        if gone {
            a.find(
                DV14,
                &desc_key(id),
                format!("DERIVED_DESC {} of a unit that does not exist or is deleted", hex(&id.0)),
            );
            a.repair.extend([
                Write::Delete(desc_key(id)),
                Write::Delete(vec_key(id)),
                Write::Delete(src_key(d.source, id)),
            ]);
            continue;
        }
        let mut loaded = Loaded { desc: d.clone(), vector: None, cause: None };
        if d.state == VectorState::Ready {
            let moved = head.as_ref().is_none_or(|h| h.head_rev != d.source.rev || h.lifecycle == Lifecycle::Deleted)
                || input_hash_of_ids(&[d.source]) != d.input_hash;
            let cause = match vectors.remove(&id) {
                _ if moved => Some(StaleCause::SourceMoved),
                None => {
                    a.find(DV12, &desc_key(id), format!("DERIVED_DESC {} is Ready without a vector", hex(&id.0)));
                    Some(StaleCause::VectorMissing)
                }
                Some(Stored::Bad(c)) => Some(c),
                Some(Stored::Good(x)) => {
                    loaded.vector = Some(x);
                    None
                }
            };
            if let Some(c) = cause {
                let stale = VectorDescriptor { state: VectorState::Stale, ..d.clone() };
                a.repair.push(Write::Put(desc_key(id), stale.encode()?));
                a.repair.push(Write::Delete(vec_key(id)));
                loaded.cause = Some(c);
            }
        }
        a.entries.insert(id, loaded);
    }
    Ok(a)
}

/// The rule a key that is not of the expected kind breaks.
fn key_rule(k: Res<DerivedKey>) -> DerivedRule {
    k.err().map_or(DV3, |e| e.rule)
}

fn hex(b: &[u8]) -> String {
    use fmt::Write as _;
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

/// The findings of `verify_units` for the persisted derived data.
pub(crate) fn verify_findings<S: Snapshot>(snap: &S, mark: Option<u32>) -> Result<Vec<DerivedFinding>, UnitError> {
    Ok(analyze(&SnapReader(snap), mark)?.findings)
}

// ------------------------------------------------------------ public API

/// A vector to persist: the exact source revision, how it was made and the
/// vector itself. The vector never appears in `Debug` output (I-37).
#[derive(Clone, PartialEq)]
pub struct VectorSpec {
    pub source: RevisionId,
    pub method: String,
    pub method_version: u32,
    /// The model profile (ADR-0018, I-25): opaque text, 1..=64 bytes.
    pub profile: String,
    pub params: BTreeMap<String, String>,
    pub vector: Vec<f32>,
}

impl fmt::Debug for VectorSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VectorSpec")
            .field("source", &self.source)
            .field("method_len", &self.method.len())
            .field("method_version", &self.method_version)
            .field("dim", &self.vector.len())
            .finish_non_exhaustive()
    }
}

impl VectorSpec {
    /// The recipe may declare the dimension as the `dim` parameter; it must be
    /// the length of the vector it produced (FRICTION 10).
    fn check_dim(&self) -> Result<(), UnitError> {
        if let Some(declared) = self.params.get("dim")
            && declared.parse::<usize>().ok() != Some(self.vector.len())
        {
            return Err(UnitError::InvalidArgument(format!(
                "recipe parameter dim is {declared:?} but the vector has {} elements",
                self.vector.len()
            )));
        }
        Ok(())
    }

    /// The descriptor and the encoded vector this spec persists.
    fn build(&self) -> Result<(VectorDescriptor, Vec<u8>), UnitError> {
        let value = encode_vector(&self.vector)?;
        let desc = VectorDescriptor {
            state: VectorState::Ready,
            source: self.source,
            method: self.method.clone(),
            method_version: self.method_version,
            profile: self.profile.clone(),
            params: self.params.clone(),
            input_hash: input_hash_of_ids(&[self.source]),
            output_hash: output_hash(&value),
        };
        desc.encode()?;
        Ok((desc, value))
    }
}

/// How a vector is made: the method and its version, the model profile and
/// the parameters that change the result. With the source revision these are
/// the identity of the datum (ADR-0020 §C.1).
#[derive(Clone, PartialEq, Eq)]
pub struct VectorRecipe {
    pub method: String,
    pub version: u32,
    /// The model profile (ADR-0018, I-25): opaque text, 1..=64 bytes.
    pub profile: String,
    pub params: BTreeMap<String, String>,
}

impl VectorRecipe {
    /// The recipe a registry descriptor was made with.
    pub fn of(d: &DerivedDescriptor) -> Self {
        VectorRecipe {
            method: d.method().to_owned(),
            version: d.version(),
            profile: d.profile().to_owned(),
            params: d.params().clone(),
        }
    }
}

impl fmt::Debug for VectorRecipe {
    /// Caller-supplied text is shown by size only (I-37).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VectorRecipe")
            .field("method_len", &self.method.len())
            .field("version", &self.version)
            .field("profile_len", &self.profile.len())
            .field("params", &self.params.len())
            .finish()
    }
}

/// Computes a vector for the current sources of a datum; `None` means it
/// could not be computed. The core runs no model: the caller supplies this.
pub type VectorCompute<'a> = &'a dyn Fn(&DerivedDescriptor, &[SourceInput]) -> Option<Vec<f32>>;

fn read_head<R: Source>(r: &R, unit: UnitId) -> Result<UnitHead, UnitError> {
    let Some(raw) = r.get(&Key::UnitHead { unit }.encode()?)? else {
        return Err(UnitError::NotFound { unit });
    };
    let head = UnitHead::decode(&raw)?;
    if head.lifecycle == Lifecycle::Deleted {
        return Err(UnitError::Deleted { unit });
    }
    Ok(head)
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Writes the three records of `spec` after checking that its source is
    /// the head of its unit as this change set sees it (§S3.1).
    fn write_vector(
        &mut self,
        spec: &VectorSpec,
    ) -> Result<(DerivedId, VectorDescriptor, Vec<f32>), DerivedStaleError> {
        self.admit_derived()?;
        let (desc, value) = spec.build()?;
        let txn = self.txn()?;
        let head = read_head(&TxnReader(txn), spec.source.unit)?;
        if head.head_rev != spec.source.rev {
            return Err(UnitError::InvalidArgument(format!(
                "revision {} of unit {} is not its head ({}): nothing is persisted over an old revision",
                spec.source.rev, spec.source.unit, head.head_rev
            ))
            .into());
        }
        let id = desc.id();
        if txn.get(&desc_key(id)).map_err(UnitError::from)?.is_some() {
            return Err(DerivedError::Duplicate(id).into());
        }
        let writes = [
            Write::Put(desc_key(id), desc.encode()?),
            Write::Put(vec_key(id), value),
            Write::Put(src_key(spec.source, id), encode_src_entry()),
        ];
        for w in &writes {
            self.write(w)?;
        }
        self.derived_ops += 1;
        Ok((id, desc, spec.vector.iter().map(|x| if *x == 0.0 { 0.0 } else { *x }).collect()))
    }

    /// The persisted descriptor `id` as this change set sees it.
    fn read_desc(&self, id: DerivedId) -> Result<VectorDescriptor, DerivedStaleError> {
        let txn = self.txn()?;
        let Some(raw) = txn.get(&desc_key(id)).map_err(UnitError::from)? else {
            return Err(DerivedError::Unknown(id).into());
        };
        Ok(VectorDescriptor::decode(&raw).map_err(UnitError::from)?)
    }
}

impl<E: TxnEngine> DerivedChangeSet<'_, E> {
    /// Persists a vector as a `Ready` derived datum, with its `0x62` entry, in
    /// this change set (and the layer mark at version 2 if this is the first).
    /// Refused with `InvalidArgument`, writing nothing, if `spec.source` is not
    /// the head of its unit in this change set (T22).
    /// Refused with `InvalidArgument` if `spec.params["dim"]` is not the length
    /// of `spec.vector`.
    pub fn put_vector(&mut self, spec: &VectorSpec) -> Result<DerivedId, DerivedStaleError> {
        spec.check_dim()?;
        let (id, desc, vector) = self.cs.write_vector(spec)?;
        self.pending.push(Pending::Put { desc: desc.to_descriptor(), vector });
        Ok(id)
    }

    /// Persists `spec` and marks `old` `Superseded` in this change set: its
    /// vector is deleted and its `0x62` entry kept (T21).
    pub fn supersede_vector(&mut self, old: DerivedId, spec: &VectorSpec) -> Result<DerivedId, DerivedStaleError> {
        let mut prev = self.cs.read_desc(old)?;
        if prev.state == VectorState::Superseded {
            return Err(DerivedError::IllegalTransition {
                id: old,
                from: DerivedState::Superseded,
                to: DerivedState::Superseded,
            }
            .into());
        }
        let (id, desc, vector) = self.cs.write_vector(spec)?;
        prev.state = VectorState::Superseded;
        self.cs.write(&Write::Put(desc_key(old), prev.encode()?))?;
        self.cs.write(&Write::Delete(vec_key(old)))?;
        self.pending.push(Pending::Put { desc: desc.to_descriptor(), vector });
        self.pending.push(Pending::Supersede { old });
        Ok(id)
    }

    /// Replaces a datum that is `Stale` in the registry by the one `spec`
    /// describes: the old records are deleted and the new ones written in this
    /// change set. The persisted descriptor may still say `Ready` (opening
    /// demotes a datum in memory without writing); a `Superseded` one is final.
    /// Nothing of `old` survives and the id of the new datum is returned.
    pub fn replace_vector(&mut self, old: DerivedId, spec: &VectorSpec) -> Result<DerivedId, DerivedStaleError> {
        let prev = self.cs.read_desc(old)?;
        if prev.state == VectorState::Superseded {
            return Err(DerivedError::IllegalTransition {
                id: old,
                from: DerivedState::Superseded,
                to: DerivedState::Building,
            }
            .into());
        }
        self.cs.admit_derived()?;
        for w in [Write::Delete(desc_key(old)), Write::Delete(vec_key(old)), Write::Delete(src_key(prev.source, old))] {
            self.cs.write(&w)?;
        }
        let (id, desc, vector) = self.cs.write_vector(spec)?;
        self.pending.push(Pending::Remove { old });
        self.pending.push(Pending::Put { desc: desc.to_descriptor(), vector });
        Ok(id)
    }
}

/// What reopening found (ADR-0020 "Reabrir").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReopenReport {
    /// Descriptors loaded `Ready`, with their vector verified.
    pub served: usize,
    /// Descriptors persisted `Ready` that are served `Stale`, and why.
    pub demoted: Vec<(DerivedId, StaleCause)>,
    /// What `verify_units` would report for the derived records.
    pub findings: Vec<DerivedFinding>,
}

/// What [`DerivedDb::repair_derived`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairReport {
    /// Records written or deleted by the repair change set.
    pub writes: usize,
    pub commit: Option<CommitInfo>,
}

impl<E: TxnEngine> DerivedDb<E> {
    /// Opens `db` and rebuilds the registry from the persisted descriptors,
    /// writing nothing (ADR-0020 "Reabrir"). Every `Ready` descriptor whose
    /// source is still the head and whose vector hashes to its output hash is
    /// `Ready` and served; any other is `Stale` and serves nothing.
    pub fn open_persisted(db: crate::UnitDb<E>) -> Result<(Self, ReopenReport), DerivedStaleError> {
        let this = DerivedDb::wrap(db);
        let report = this.load_persisted()?;
        Ok((this, report))
    }

    fn load_persisted(&self) -> Result<ReopenReport, DerivedStaleError> {
        let snap = self.db.snapshot()?;
        let mark = snap.layer_version()?;
        let a = analyze(&SnapReader(&snap.snap), mark)?;
        let mut report = ReopenReport { findings: a.findings, ..ReopenReport::default() };
        let mut inner = self.lock();
        let old: Vec<DerivedId> = inner.persisted.iter().copied().collect();
        for id in old {
            inner.forget(&id);
        }
        for (id, l) in a.entries {
            let state = match (l.desc.state, l.cause) {
                (VectorState::Ready, Some(c)) => {
                    report.demoted.push((id, c));
                    DerivedState::Stale
                }
                (s, _) => s.to_derived(),
            };
            let reg_id = inner.registry.register(l.desc.to_descriptor())?;
            inner.registry.transition(&reg_id, state)?;
            inner.persisted.insert(reg_id);
            if let Some(v) = l.vector {
                report.served += 1;
                inner.vectors.insert(reg_id, v);
            }
            debug_assert_eq!(reg_id, id);
        }
        Ok(report)
    }

    /// The vector of a `Ready` persisted datum, as checked when it was loaded
    /// or committed.
    pub fn vector(&self, id: &DerivedId) -> Option<Vec<f32>> {
        self.lock().vectors.get(id).cloned()
    }

    /// Computes and persists the vector of `unit` at its current revision.
    pub fn build_vector(
        &self,
        unit: UnitId,
        recipe: &VectorRecipe,
        meta: ChangeMeta,
        compute: VectorCompute<'_>,
    ) -> Result<DerivedId, DerivedStaleError> {
        let input = self.read_input(unit)?.ok_or(DerivedStaleError::MissingSource(unit))?;
        let spec = self.compute_spec(&[input], recipe, compute)?;
        let mut cs = self.begin_changeset(meta)?;
        let id = cs.put_vector(&spec)?;
        cs.commit()?;
        Ok(id)
    }

    /// Recomputes a `Stale` persisted datum from its source as it is now and
    /// replaces it in one change set. The result is what [`build_vector`]
    /// gives over the same source (I-24).
    ///
    /// [`build_vector`]: DerivedDb::build_vector
    pub fn rebuild_vector(
        &self,
        id: &DerivedId,
        meta: ChangeMeta,
        compute: VectorCompute<'_>,
    ) -> Result<DerivedId, DerivedStaleError> {
        let old = {
            let inner = self.lock();
            let state = inner.registry.state(id).ok_or(DerivedError::Unknown(*id))?;
            if !inner.persisted.contains(id) || state != DerivedState::Stale {
                return Err(DerivedError::IllegalTransition { id: *id, from: state, to: DerivedState::Building }.into());
            }
            inner.registry.descriptor(id).cloned().ok_or(DerivedError::Unknown(*id))?
        };
        let inputs = self.valid_inputs(&old)?;
        if inputs.is_empty() {
            return Err(DerivedStaleError::NoValidSources(*id));
        }
        let spec = self.compute_spec(&inputs, &VectorRecipe::of(&old), compute)?;
        let mut cs = self.begin_changeset(meta)?;
        let new = cs.replace_vector(*id, &spec)?;
        cs.commit()?;
        Ok(new)
    }

    /// Persists the datum for a new method, version or profile over the
    /// still-valid source of `old`, and marks `old` `Superseded`, in one
    /// change set.
    pub fn supersede_vector(
        &self,
        old: &DerivedId,
        recipe: &VectorRecipe,
        meta: ChangeMeta,
        compute: VectorCompute<'_>,
    ) -> Result<DerivedId, DerivedStaleError> {
        let prev = {
            let inner = self.lock();
            if !inner.persisted.contains(old) {
                return Err(DerivedError::Unknown(*old).into());
            }
            inner.registry.descriptor(old).cloned().ok_or(DerivedError::Unknown(*old))?
        };
        let inputs = self.valid_inputs(&prev)?;
        if inputs.is_empty() {
            return Err(DerivedStaleError::NoValidSources(*old));
        }
        let spec = self.compute_spec(&inputs, recipe, compute)?;
        let mut cs = self.begin_changeset(meta)?;
        let id = cs.supersede_vector(*old, &spec)?;
        cs.commit()?;
        Ok(id)
    }

    fn compute_spec(
        &self,
        inputs: &[SourceInput],
        recipe: &VectorRecipe,
        compute: VectorCompute<'_>,
    ) -> Result<VectorSpec, DerivedStaleError> {
        let desc = Self::describe(inputs, &recipe.method, recipe.version, recipe.params.clone())?
            .with_profile(recipe.profile.clone());
        let Some(vector) = compute(&desc, inputs) else {
            return Err(DerivedStaleError::ComputeFailed(desc.id()));
        };
        let source = inputs.first().map(|i| i.id).ok_or(DerivedError::EmptySources)?;
        Ok(VectorSpec {
            source,
            method: recipe.method.clone(),
            method_version: recipe.version,
            profile: recipe.profile.clone(),
            params: recipe.params.clone(),
            vector,
        })
    }

    /// The explicit repair (ADR-0020 "Reparación"): one change set of purpose
    /// `Derived` that deletes what has no valid descriptor, rewrites the
    /// `0x62` entries, removes the derived data of a source that is gone and
    /// persists the `Stale` state of what is no longer `Ready`. Opening never
    /// repairs. A database without damage commits nothing.
    pub fn repair_derived(&self, meta: ChangeMeta) -> Result<RepairReport, DerivedStaleError> {
        let mut cs = self.db.begin_changeset(meta)?;
        let mark = cs.layer_version;
        let a = analyze(&TxnReader(cs.txn()?), Some(mark))?;
        if a.repair.is_empty() {
            cs.abort();
            return Ok(RepairReport { writes: 0, commit: None });
        }
        let writes = a.repair.len();
        cs.admit_derived()?;
        for w in &a.repair {
            cs.write(w)?;
        }
        cs.derived_ops += 1;
        let commit = {
            let _guard = self.lock();
            cs.commit()?
        };
        self.load_persisted()?;
        Ok(RepairReport { writes, commit: Some(commit) })
    }
}
