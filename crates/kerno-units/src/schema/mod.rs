//! Typed schemas per unit type and user-defined secondary indexes (Phase C,
//! ADR-0021).
//!
//! # Model
//!
//! A unit type may have a [`Schema`]: named, typed fields, each optionally
//! `required`, `unique` or `indexed`. The payload of every `Active` unit of
//! that type is then a [`Document`] (a canonical binary encoding of those
//! fields) that the schema accepts. A type without a schema is untouched: its
//! payloads stay opaque bytes and a database that holds no schema is
//! byte-for-byte what it was before this module.
//!
//! # Writes
//!
//! Validation and index maintenance run inside the operation that writes the
//! unit's head ([`ChangeSet::plan_schema`](crate::db::ChangeSet), the path of
//! [`crate::fulltext`] and [`crate::type_count`]): planned by reads, before the
//! first write of the operation. So
//!
//! * a write the schema refuses ([`UnitError::SchemaViolation`]) stores
//!   nothing and leaves the change set usable; dropping the change set after a
//!   refusal stores nothing of it;
//! * an index never disagrees with the units it describes, across a crash or a
//!   reopen (one engine transaction), and a rebuilt index equals a fresh one;
//! * `create`, `update` and `restore` validate the new payload; `archive` and
//!   `delete` remove the unit's entries; `unarchive` validates again and
//!   re-checks uniqueness, because another unit may have taken the value
//!   meanwhile. Uniqueness is among `Active` units only.
//!
//! [`ChangeSet::define_schema`](crate::db::ChangeSet), `create_index` and
//! `drop_index` write a definition and backfill or remove its entries in the
//! change set that carries them; a type whose existing `Active` units do not
//! satisfy the schema (or repeat a unique value) is refused whole.
//!
//! # Records
//!
//! `record ::= 1 ‖ n: u16 BE ‖ n × field`, `field ::= name_len: u8 ‖ name ‖
//! type: u8 ‖ value`, fields in strictly ascending name order (one encoding
//! per record). Values: `Bool` one byte (0 or 1), `Int` an `i64` BE, `Float` an
//! `f64` BE (never NaN), `Text` and `Bytes` a `u32` BE length and the bytes
//! (`Text` is UTF-8).
//!
//! # Stored records
//!
//! * `SCHEMA` `0x78 ‖ type_len: u8 ‖ unit_type` → `envelope[2] ‖ n: u16 BE ‖ n
//!   × (name_len: u8 ‖ name ‖ type: u8 ‖ flags: u8)` with flags bit 0
//!   `required`, bit 1 `unique`, bit 2 `indexed`, bit 3 `covering` (`unique` and
//!   `covering` imply `indexed`; bit 3 needs layer 13, `units-v13.md`).
//! * `FIELD_INDEX` `0x79 ‖ type_len ‖ unit_type ‖ name_len ‖ name ‖ value ‖
//!   unit[16]` → `envelope[2]`: one entry per `Active` unit that has the
//!   field, for every indexed field. An entry of a `covering` field whose unit's
//!   payload is at most `INLINE_PAYLOAD_MAX` bytes is `envelope[2] ‖ payload`
//!   with envelope flag bit 0 set (layer 13), rewritten whenever the payload changes. `value` is order-preserving and
//!   self-delimiting: `Bool` one byte; `Int` the `i64` BE with its sign bit
//!   flipped; `Float` the `f64` bits BE, all bits flipped when negative and the
//!   sign bit set otherwise (`-0.0` is stored as `0.0`); `Text` and `Bytes` the
//!   bytes with each `0x00` written `0x00 0xFF`, then `0x00 0x00`. A range of
//!   keys is therefore the units in `(value, unit id)` order.
//!
//! The first `0x78` record raises the layer mark to 6 in its own transaction
//! (`LAYER_VERSION_SCHEMAS`, ADR-0021, `docs/file-format/units-v6.md`), so a
//! build that predates schemas refuses the database (UNIT-16) instead of
//! writing typed units unchecked. `0x77` is assigned to type options by ADR-0022 (layer 7). An indexed `Text` or `Bytes` value whose encoding
//! exceeds [`MAX_INDEXED_VALUE_LEN`] bytes is refused, so a key always fits the
//! engine's key limit.
//!
//! `ponytail:` `define_schema` and `create_index` read every `Active` unit of
//! the type into memory and write all the entries in one change set, so they
//! are bound by the engine's batch limits (`ChangeSetTooLarge`, nothing
//! stored). A schema has no migration: it is defined once, and only the
//! `indexed` flag (`create_index`, `drop_index`) and the `covering` flag (`define_schema`
//! again with only that flag changed) of a field change afterwards.
//!
//! `ponytail:` a read checks every unit it returns against its entry but cannot
//! see an entry that is missing; that is `verify_units` (UNIT-50).

mod check;
mod read;
#[cfg(test)]
mod tests;
mod write;

pub use write::{UpsertOutcome, Upserted, upsert_key};

use std::collections::BTreeMap;

use crate::error::{DecodeError, UnitError, UnitRule};
use crate::types::UnitId;

pub(crate) use check::Check;

/// Key tags of the schema records.
pub const TAG_SCHEMA: u8 = 0x78;
pub const TAG_FIELD_INDEX: u8 = 0x79;

/// Version of the records of this module (`envelope_version`, and the first
/// byte of a [`Document`]).
const RECORD_VERSION: u8 = 1;
/// Fields per schema and per record.
pub const MAX_FIELDS: usize = 64;
/// Bytes of a field name (UTF-8, no NUL).
pub const MAX_FIELD_NAME_LEN: usize = 64;
/// Bytes of the encoded value of an indexed field.
pub const MAX_INDEXED_VALUE_LEN: usize = 256;
/// Bytes of text (or bytes) an indexed field can hold: the encoding of `Text` and `Bytes` adds a
/// two-byte terminator, so 254 fit under [`MAX_INDEXED_VALUE_LEN`]. A `0x00` byte is written as
/// two, so a value with `n` of them holds `n` fewer.
pub const MAX_INDEXED_TEXT_LEN: usize = MAX_INDEXED_VALUE_LEN - 2;
const MAX_TYPE_LEN: usize = crate::codec::MAX_UNIT_TYPE_LEN as usize;

// -------------------------------------------------------------- field types

/// The type of a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldType {
    Bool,
    Int,
    Float,
    Text,
    Bytes,
}

impl FieldType {
    fn to_byte(self) -> u8 {
        match self {
            FieldType::Bool => 1,
            FieldType::Int => 2,
            FieldType::Float => 3,
            FieldType::Text => 4,
            FieldType::Bytes => 5,
        }
    }

    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            1 => FieldType::Bool,
            2 => FieldType::Int,
            3 => FieldType::Float,
            4 => FieldType::Text,
            5 => FieldType::Bytes,
            _ => return None,
        })
    }
}

/// The value of a field.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Bool(bool),
    Int(i64),
    /// Never NaN: a record holding one is refused.
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl FieldValue {
    pub fn field_type(&self) -> FieldType {
        match self {
            FieldValue::Bool(_) => FieldType::Bool,
            FieldValue::Int(_) => FieldType::Int,
            FieldValue::Float(_) => FieldType::Float,
            FieldValue::Text(_) => FieldType::Text,
            FieldValue::Bytes(_) => FieldType::Bytes,
        }
    }

    /// The order-preserving, self-delimiting encoding an index key holds, or
    /// `None` for a value no index can hold (NaN).
    pub(crate) fn index_bytes(&self) -> Option<Vec<u8>> {
        Some(match self {
            FieldValue::Bool(b) => vec![u8::from(*b)],
            FieldValue::Int(i) => int_index(*i).to_vec(),
            FieldValue::Float(f) if f.is_nan() => return None,
            FieldValue::Float(f) => float_index(*f).to_vec(),
            FieldValue::Text(s) => escaped(s.as_bytes()),
            FieldValue::Bytes(b) => escaped(b),
        })
    }

    /// The length of [`index_bytes`](Self::index_bytes), `None` for NaN.
    pub(crate) fn index_len(&self) -> Option<usize> {
        Some(match self {
            FieldValue::Bool(_) => 1,
            FieldValue::Int(_) => 8,
            FieldValue::Float(f) if f.is_nan() => return None,
            FieldValue::Float(_) => 8,
            FieldValue::Text(s) => escaped_len(s.as_bytes()),
            FieldValue::Bytes(b) => escaped_len(b),
        })
    }

    /// Appends [`index_bytes`](Self::index_bytes) to `out`; `false` (nothing appended) for NaN.
    pub(crate) fn write_index(&self, out: &mut Vec<u8>) -> bool {
        match self {
            FieldValue::Bool(b) => out.push(u8::from(*b)),
            FieldValue::Int(i) => out.extend_from_slice(&int_index(*i)),
            FieldValue::Float(f) if f.is_nan() => return false,
            FieldValue::Float(f) => out.extend_from_slice(&float_index(*f)),
            FieldValue::Text(s) => escaped_into(s.as_bytes(), out),
            FieldValue::Bytes(b) => escaped_into(b, out),
        }
        true
    }
}

/// The index encoding of an `Int`: its bits with the sign flipped, so it sorts as a number.
fn int_index(i: i64) -> [u8; 8] {
    ((i as u64) ^ (1 << 63)).to_be_bytes()
}

/// The index encoding of a `Float` that is not NaN: sorts as a number, `-0.0` as `0.0`.
fn float_index(f: f64) -> [u8; 8] {
    let bits = if f == 0.0 { 0.0f64 } else { f }.to_bits();
    let ordered = if bits >> 63 == 1 { !bits } else { bits | (1 << 63) };
    ordered.to_be_bytes()
}

/// `0x00` as `0x00 0xFF`, then the terminator `0x00 0x00`: sorts like the bytes
/// and no encoding is a prefix of another.
fn escaped(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(escaped_len(bytes));
    escaped_into(bytes, &mut out);
    out
}

fn escaped_len(bytes: &[u8]) -> usize {
    bytes.len() + memchr::memchr_iter(0, bytes).count() + 2
}

/// [`escaped`], appended to `out`: the runs between `0x00` bytes copied whole.
fn escaped_into(bytes: &[u8], out: &mut Vec<u8>) {
    let mut rest = bytes;
    while let Some(i) = memchr::memchr(0, rest) {
        out.extend_from_slice(&rest[..=i]);
        out.push(0xFF);
        rest = &rest[i + 1..];
    }
    out.extend_from_slice(rest);
    out.extend_from_slice(&[0, 0]);
}

// ------------------------------------------------------------------ records

/// The payload of a unit whose type has a schema: named field values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document(BTreeMap<String, FieldValue>);

impl Document {
    pub fn new() -> Self {
        Document::default()
    }

    /// This record with `name` set to `value`.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: FieldValue) -> Self {
        self.0.insert(name.into(), value);
        self
    }

    pub fn get(&self, name: &str) -> Option<&FieldValue> {
        self.0.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &FieldValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// The canonical bytes of the record, to use as a unit's payload.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a name outside 1..=64 bytes or with a NUL, more
    /// than [`MAX_FIELDS`] fields, a NaN or a value over 4 GiB.
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let bad = |m: String| UnitError::InvalidArgument(format!("record: {m}"));
        if self.0.len() > MAX_FIELDS {
            return Err(bad(format!("{} fields exceed {MAX_FIELDS}", self.0.len())));
        }
        let mut out = vec![RECORD_VERSION];
        out.extend_from_slice(&(self.0.len() as u16).to_be_bytes());
        for (name, value) in &self.0 {
            check_name(name).map_err(bad)?;
            out.push(name.len() as u8);
            out.extend_from_slice(name.as_bytes());
            out.push(value.field_type().to_byte());
            match value {
                FieldValue::Bool(b) => out.push(u8::from(*b)),
                FieldValue::Int(i) => out.extend_from_slice(&i.to_be_bytes()),
                FieldValue::Float(f) if f.is_nan() => return Err(bad(format!("field {name:?} is NaN"))),
                FieldValue::Float(f) => out.extend_from_slice(&f.to_be_bytes()),
                FieldValue::Text(s) => put_bytes(&mut out, s.as_bytes()).map_err(bad)?,
                FieldValue::Bytes(b) => put_bytes(&mut out, b).map_err(bad)?,
            }
        }
        Ok(out)
    }

    /// The record `bytes` encode.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when `bytes` are not exactly a canonical record.
    pub fn decode(bytes: &[u8]) -> Result<Document, UnitError> {
        Document::parse(bytes).map_err(|m| UnitError::InvalidArgument(format!("record: {m}")))
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Document, String> {
        let mut map = BTreeMap::new();
        walk_record(bytes, |name, value| {
            map.insert(name.to_owned(), value.to_owned_value());
        })?;
        Ok(Document(map))
    }

    /// Reads the document in `bytes`, handing each field to `visit` in name order, without
    /// building a [`Document`]: for a caller that turns the fields into values of its own (a
    /// language binding making a dictionary). `visit` may have been called for some fields
    /// before an error is found in a later one.
    ///
    /// # Errors
    ///
    /// As [`decode`](Self::decode).
    pub fn visit<'a>(bytes: &'a [u8], visit: impl FnMut(&'a str, ValueRef<'a>)) -> Result<(), UnitError> {
        walk_record(bytes, visit).map_err(|m| UnitError::InvalidArgument(format!("record: {m}")))
    }

    /// Whether `bytes` are a canonical record whose `field` has the index encoding `enc`:
    /// `Document::parse(bytes)` then `get(field)` then `index_bytes() == enc`, without building
    /// the document. A lookup asks this of every unit an index entry names.
    pub(crate) fn field_encodes_to(bytes: &[u8], field: &str, enc: &[u8]) -> bool {
        let mut agrees = false;
        let walked = walk_record(bytes, |name, value| {
            if name == field {
                agrees = value.index_eq(enc);
            }
        });
        walked.is_ok() && agrees
    }
}

/// A field value inside the bytes of a document: what [`Document::visit`] hands out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ValueRef<'a> {
    Bool(bool),
    Int(i64),
    /// Never NaN.
    Float(f64),
    Text(&'a str),
    Bytes(&'a [u8]),
}

impl ValueRef<'_> {
    fn to_owned_value(self) -> FieldValue {
        match self {
            ValueRef::Bool(b) => FieldValue::Bool(b),
            ValueRef::Int(i) => FieldValue::Int(i),
            ValueRef::Float(f) => FieldValue::Float(f),
            ValueRef::Text(s) => FieldValue::Text(s.to_owned()),
            ValueRef::Bytes(b) => FieldValue::Bytes(b.to_vec()),
        }
    }

    /// [`FieldValue::index_bytes`] of this value (never NaN, so always some).
    pub(crate) fn index_bytes(self) -> Vec<u8> {
        match self {
            ValueRef::Bool(b) => vec![u8::from(b)],
            ValueRef::Int(i) => int_index(i).to_vec(),
            ValueRef::Float(f) => float_index(f).to_vec(),
            ValueRef::Text(s) => escaped(s.as_bytes()),
            ValueRef::Bytes(b) => escaped(b),
        }
    }

    /// Whether [`FieldValue::index_bytes`] of this value is `enc`.
    fn index_eq(self, enc: &[u8]) -> bool {
        match self {
            ValueRef::Bool(b) => enc == [u8::from(b)],
            ValueRef::Int(i) => enc == int_index(i),
            ValueRef::Float(f) => enc == float_index(f),
            ValueRef::Text(s) => escaped_eq(s.as_bytes(), enc),
            ValueRef::Bytes(b) => escaped_eq(b, enc),
        }
    }
}

/// Whether [`escaped`] of `bytes` is `enc`.
fn escaped_eq(bytes: &[u8], enc: &[u8]) -> bool {
    let mut rest = enc;
    for &b in bytes {
        let n = if b == 0 { 2 } else { 1 };
        if rest.len() < n || rest[0] != b || (b == 0 && rest[1] != 0xFF) {
            return false;
        }
        rest = &rest[n..];
    }
    rest == [0, 0]
}

/// Reads a record, handing each field to `visit` in order. An error is what
/// [`Document::parse`] says of bytes that are not a canonical record.
pub(crate) fn walk_record<'a>(bytes: &'a [u8], mut visit: impl FnMut(&'a str, ValueRef<'a>)) -> Result<(), String> {
    let mut r = Cursor(bytes);
    if r.take(1)? != [RECORD_VERSION] {
        return Err("not a record (unknown version byte)".into());
    }
    let n = u16::from_be_bytes(r.array()?);
    if usize::from(n) > MAX_FIELDS {
        return Err(format!("{n} fields exceed {MAX_FIELDS}"));
    }
    let mut last: Option<&str> = None;
    for _ in 0..n {
        let len = usize::from(r.take(1)?[0]);
        let name = std::str::from_utf8(r.take(len)?).map_err(|_| "a field name is not UTF-8".to_owned())?;
        check_name(name)?;
        if last.is_some_and(|l| l >= name) {
            return Err("field names are not in strictly ascending order".into());
        }
        let ty = FieldType::from_byte(r.take(1)?[0]).ok_or_else(|| format!("field {name:?} has an unknown type"))?;
        let value = match ty {
            FieldType::Bool => match r.take(1)?[0] {
                0 => ValueRef::Bool(false),
                1 => ValueRef::Bool(true),
                _ => return Err(format!("field {name:?} is a Bool that is neither 0 nor 1")),
            },
            FieldType::Int => ValueRef::Int(i64::from_be_bytes(r.array()?)),
            FieldType::Float => {
                let f = f64::from_be_bytes(r.array()?);
                if f.is_nan() {
                    return Err(format!("field {name:?} is NaN"));
                }
                ValueRef::Float(f)
            }
            FieldType::Text | FieldType::Bytes => {
                let len = u32::from_be_bytes(r.array()?) as usize;
                let raw = r.take(len)?;
                if ty == FieldType::Bytes {
                    ValueRef::Bytes(raw)
                } else {
                    ValueRef::Text(std::str::from_utf8(raw).map_err(|_| format!("field {name:?} is not UTF-8"))?)
                }
            }
        };
        last = Some(name);
        visit(name, value);
    }
    if !r.0.is_empty() {
        return Err(format!("{} trailing bytes", r.0.len()));
    }
    Ok(())
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) -> Result<(), String> {
    let n = u32::try_from(b.len()).map_err(|_| "a value is over 4 GiB".to_owned())?;
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(b);
    Ok(())
}

fn check_name(name: &str) -> Result<(), String> {
    if (1..=MAX_FIELD_NAME_LEN).contains(&name.len()) && !name.contains('\0') {
        Ok(())
    } else {
        Err(format!("a field name of {} bytes is outside 1..={MAX_FIELD_NAME_LEN} or has a NUL", name.len()))
    }
}

/// A reader over `&[u8]` that fails with a message instead of panicking.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("the bytes end inside a value".into());
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
}

// ------------------------------------------------------------------ schemas

/// One field of a [`Schema`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDef {
    pub name: String,
    pub ty: FieldType,
    /// Every record must have the field.
    pub required: bool,
    /// No two `Active` units of the type hold the same value. Implies `indexed`.
    pub unique: bool,
    /// Entries of this field are kept, for [`UnitSnapshot::find_by_field`](crate::UnitSnapshot).
    pub indexed: bool,
    /// Each entry of this indexed field also holds its unit's payload, when the payload is one
    /// the unit's head stores inline (at most [`INLINE_PAYLOAD_MAX`](crate::codec::INLINE_PAYLOAD_MAX)
    /// bytes), so [`UnitSnapshot::find_payloads_by_field`](crate::UnitSnapshot) answers from the
    /// entries alone (layer 13, `docs/file-format/units-v13.md`). Only valid with `indexed`.
    pub covering: bool,
}

impl FieldDef {
    /// An optional, non-unique, unindexed field.
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        FieldDef { name: name.into(), ty, required: false, unique: false, indexed: false, covering: false }
    }

    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Unique, and so indexed.
    #[must_use]
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self.indexed = true;
        self
    }

    #[must_use]
    pub fn indexed(mut self) -> Self {
        self.indexed = true;
        self
    }

    /// Covering: its entries carry the unit's payload (see [`covering`](Self::covering)). The
    /// field must also be `indexed` (or `unique`).
    #[must_use]
    pub fn covering(mut self) -> Self {
        self.covering = true;
        self
    }

    fn flags(&self) -> u8 {
        u8::from(self.required)
            | u8::from(self.unique) << 1
            | u8::from(self.indexed) << 2
            | u8::from(self.covering) << 3
    }
}

/// The fields a unit type's records may and must have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schema {
    /// In ascending name order, names unique.
    fields: Vec<FieldDef>,
}

impl Schema {
    /// A schema of `fields` (sorted by name; `unique` fields are indexed).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a repeated or invalid name or more than
    /// [`MAX_FIELDS`] fields.
    pub fn new(mut fields: Vec<FieldDef>) -> Result<Self, UnitError> {
        let bad = |m: String| UnitError::InvalidArgument(format!("schema: {m}"));
        if fields.len() > MAX_FIELDS {
            return Err(bad(format!("{} fields exceed {MAX_FIELDS}", fields.len())));
        }
        fields.sort_by(|a, b| a.name.cmp(&b.name));
        for f in &mut fields {
            check_name(&f.name).map_err(bad)?;
            f.indexed |= f.unique;
        }
        if let Some(w) = fields.windows(2).find(|w| w[0].name == w[1].name) {
            return Err(bad(format!("field {:?} is declared twice", w[0].name)));
        }
        if let Some(f) = fields.iter().find(|f| f.covering && !f.indexed) {
            return Err(bad(format!("field {:?} is covering but not indexed", f.name)));
        }
        Ok(Schema { fields })
    }

    pub fn fields(&self) -> &[FieldDef] {
        &self.fields
    }

    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Whether a field of this schema is covering (layer 13).
    pub fn has_covering(&self) -> bool {
        self.fields.iter().any(|f| f.covering)
    }

    /// Whether `record` is one this schema accepts: no undeclared field, every
    /// value of its field's type, every required field present. The message
    /// names fields, never values.
    pub fn validate(&self, record: &Document) -> Result<(), String> {
        for (name, value) in record.iter() {
            let Some(def) = self.field(name) else {
                return Err(format!("field {name:?} is not declared"));
            };
            if value.field_type() != def.ty {
                return Err(format!("field {name:?} is {:?}, the schema says {:?}", value.field_type(), def.ty));
            }
        }
        match self.fields.iter().find(|f| f.required && record.get(&f.name).is_none()) {
            Some(f) => Err(format!("required field {:?} is missing", f.name)),
            None => Ok(()),
        }
    }

    /// The index entries a valid `record` has: `(field position, encoded
    /// value)` for every indexed field it holds.
    pub(crate) fn entries(&self, record: &Document) -> Result<Vec<(usize, Vec<u8>)>, String> {
        let mut out = Vec::new();
        for (i, def) in self.fields.iter().enumerate().filter(|(_, d)| d.indexed) {
            let Some(value) = record.get(&def.name) else {
                continue;
            };
            let Some(enc) = value.index_bytes().filter(|e| e.len() <= MAX_INDEXED_VALUE_LEN) else {
                return Err(format!(
                    "the value of indexed field {:?} is too long to index: at most {MAX_INDEXED_TEXT_LEN} bytes of \
                     text or bytes (each 0x00 byte counts twice)",
                    def.name
                ));
            };
            out.push((i, enc));
        }
        Ok(out)
    }

    fn encode(&self) -> Vec<u8> {
        let mut v = vec![RECORD_VERSION, 0];
        v.extend_from_slice(&(self.fields.len() as u16).to_be_bytes());
        for f in &self.fields {
            v.push(f.name.len() as u8);
            v.extend_from_slice(f.name.as_bytes());
            v.push(f.ty.to_byte());
            v.push(f.flags());
        }
        v
    }

    fn decode(v: &[u8]) -> Result<Schema, DecodeError> {
        let e = |rule, m: &str| DecodeError::new(rule, format!("schema record: {m}"));
        let (Some(&version), Some(&flags)) = (v.first(), v.get(1)) else {
            return Err(e(UnitRule::Unit9, "fewer bytes than its envelope"));
        };
        match version {
            0 => return Err(e(UnitRule::Unit6, "envelope_version 0")),
            RECORD_VERSION => {}
            _ => return Err(e(UnitRule::Unit7, "a newer envelope_version")),
        }
        if flags != 0 {
            return Err(e(UnitRule::Unit8, "reserved_flags set"));
        }
        let mut r = Cursor(&v[2..]);
        let short = |_: String| e(UnitRule::Unit9, "ends before its last field");
        let n = u16::from_be_bytes(r.array().map_err(short)?);
        let mut fields = Vec::new();
        for _ in 0..n {
            let len = usize::from(r.take(1).map_err(short)?[0]);
            let name = std::str::from_utf8(r.take(len).map_err(short)?)
                .map_err(|_| e(UnitRule::Unit12, "a field name is not UTF-8"))?
                .to_owned();
            let [ty, flags] = r.array().map_err(short)?;
            let ty = FieldType::from_byte(ty).ok_or_else(|| e(UnitRule::Unit12, "an unknown field type"))?;
            // Bit 3, covering, implies indexed like bit 1, unique.
            if flags > 0b1111 || (flags & 0b1010 != 0 && flags & 0b100 == 0) {
                return Err(e(UnitRule::Unit12, "field flags no writer produces"));
            }
            fields.push(FieldDef {
                name,
                ty,
                required: flags & 1 != 0,
                unique: flags & 0b10 != 0,
                indexed: flags & 0b100 != 0,
                covering: flags & 0b1000 != 0,
            });
        }
        if !r.0.is_empty() {
            return Err(e(UnitRule::Unit10, "trailing bytes"));
        }
        let ordered = fields.len() <= MAX_FIELDS
            && fields.windows(2).all(|w| w[0].name < w[1].name)
            && fields.iter().all(|f| check_name(&f.name).is_ok());
        if !ordered {
            return Err(e(UnitRule::Unit12, "field names are invalid or not in ascending order"));
        }
        Ok(Schema { fields })
    }
}

// --------------------------------------------------------------------- keys

fn type_prefix(tag: u8, unit_type: &str) -> Vec<u8> {
    let mut k = vec![tag, unit_type.len() as u8];
    k.extend_from_slice(unit_type.as_bytes());
    k
}

/// The key of the schema record of `unit_type`.
pub(crate) fn schema_key(unit_type: &str) -> Vec<u8> {
    type_prefix(TAG_SCHEMA, unit_type)
}

/// The key prefix of every entry of one field, up to where its values start.
pub(crate) fn field_prefix(unit_type: &str, field: &str) -> Vec<u8> {
    field_prefix_with(unit_type, field, 0)
}

/// [`field_prefix`] in a buffer with room for `more` bytes after it.
pub(crate) fn field_prefix_with(unit_type: &str, field: &str, more: usize) -> Vec<u8> {
    let mut k = Vec::with_capacity(3 + unit_type.len() + field.len() + more);
    k.extend_from_slice(&[TAG_FIELD_INDEX, unit_type.len() as u8]);
    k.extend_from_slice(unit_type.as_bytes());
    k.push(field.len() as u8);
    k.extend_from_slice(field.as_bytes());
    k
}

pub(crate) fn entry_key(unit_type: &str, field: &str, value: &[u8], unit: UnitId) -> Vec<u8> {
    let mut k = field_prefix(unit_type, field);
    k.extend_from_slice(value);
    k.extend_from_slice(&unit.0);
    k
}

/// `[prefix, end)` of every key that starts with `prefix`.
pub(crate) fn prefix_bounds(prefix: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut end = prefix.to_vec();
    past_prefix(&mut end);
    (prefix.to_vec(), end)
}

/// Turns `prefix` into the first key past every key that starts with it (the end of
/// [`prefix_bounds`]).
pub(crate) fn past_prefix(k: &mut Vec<u8>) {
    while k.last() == Some(&0xFF) {
        k.pop();
    }
    if let Some(last) = k.last_mut() {
        *last += 1;
    }
}

pub(crate) fn decode_schema_key(key: &[u8]) -> Result<String, DecodeError> {
    let ok = key.len() >= 3 && key.len() == 2 + usize::from(key[1]) && usize::from(key[1]) <= MAX_TYPE_LEN;
    match key.get(2..).map(std::str::from_utf8) {
        Some(Ok(t)) if ok => Ok(t.to_owned()),
        _ if ok => Err(DecodeError::new(UnitRule::Unit12, "schema key unit_type is not UTF-8")),
        _ => Err(DecodeError::new(
            UnitRule::Unit3,
            format!("schema key of {} bytes is not tag, length, type", key.len()),
        )),
    }
}

/// `(unit_type, field, encoded value, unit)` of an entry key.
pub(crate) fn decode_entry_key(key: &[u8]) -> Result<(String, String, Vec<u8>, UnitId), DecodeError> {
    let (t, f, enc, unit) = entry_key_parts(key)?;
    Ok((t.to_owned(), f.to_owned(), enc.to_vec(), unit))
}

/// [`decode_entry_key`] without copying the parts out of the key.
pub(crate) fn entry_key_parts(key: &[u8]) -> Result<(&str, &str, &[u8], UnitId), DecodeError> {
    let shape =
        || DecodeError::new(UnitRule::Unit3, format!("field index key of {} bytes has no valid shape", key.len()));
    let tlen = usize::from(*key.get(1).ok_or_else(shape)?);
    let flen_at = 2 + tlen;
    let flen = usize::from(*key.get(flen_at).ok_or_else(shape)?);
    let value_at = flen_at + 1 + flen;
    if tlen == 0 || tlen > MAX_TYPE_LEN || flen == 0 || flen > MAX_FIELD_NAME_LEN || key.len() < value_at + 1 + 16 {
        return Err(shape());
    }
    let text = |b| {
        std::str::from_utf8(b).map_err(|_| DecodeError::new(UnitRule::Unit12, "field index key names are not UTF-8"))
    };
    let unit_at = key.len() - 16;
    let mut unit = [0u8; 16];
    unit.copy_from_slice(&key[unit_at..]);
    Ok((text(&key[2..flen_at])?, text(&key[flen_at + 1..value_at])?, &key[value_at..unit_at], UnitId(unit)))
}

/// The envelope flag of an entry that carries its unit's payload (layer 13).
pub(crate) const ENTRY_PAYLOAD: u8 = 1;

/// An entry's value: its envelope and nothing else.
pub(crate) fn entry_value() -> Vec<u8> {
    vec![RECORD_VERSION, 0]
}

/// The value of an entry of `def` for a unit whose payload is `payload`: for a covering field and a
/// payload the head stores inline, the envelope with [`ENTRY_PAYLOAD`] and the payload; otherwise
/// [`entry_value`].
pub(crate) fn entry_value_for(def: &FieldDef, payload: Option<&[u8]>) -> Vec<u8> {
    match payload {
        Some(p) if def.covering && p.len() <= crate::codec::INLINE_PAYLOAD_MAX as usize => {
            let mut v = Vec::with_capacity(2 + p.len());
            v.extend_from_slice(&[RECORD_VERSION, ENTRY_PAYLOAD]);
            v.extend_from_slice(p);
            v
        }
        _ => entry_value(),
    }
}

/// The payload an entry value carries (`None`: the envelope only). The value must have passed
/// [`check_entry_value`].
pub(crate) fn entry_payload(v: &[u8]) -> Option<&[u8]> {
    match v {
        [RECORD_VERSION, ENTRY_PAYLOAD, rest @ ..] => Some(rest),
        _ => None,
    }
}

pub(crate) fn check_entry_value(v: &[u8]) -> Result<(), DecodeError> {
    let e = |rule, m: &str| DecodeError::new(rule, format!("field index record: {m}"));
    match v {
        [] | [_] => Err(e(UnitRule::Unit9, "fewer bytes than its envelope")),
        [0, ..] => Err(e(UnitRule::Unit6, "envelope_version 0")),
        [RECORD_VERSION, 0] => Ok(()),
        [RECORD_VERSION, ENTRY_PAYLOAD, rest @ ..] if rest.len() <= crate::codec::INLINE_PAYLOAD_MAX as usize => Ok(()),
        [RECORD_VERSION, ENTRY_PAYLOAD, ..] => Err(e(UnitRule::Unit10, "a payload longer than an inline one")),
        [RECORD_VERSION, 0, ..] => Err(e(UnitRule::Unit10, "trailing bytes")),
        [RECORD_VERSION, _, ..] => Err(e(UnitRule::Unit8, "reserved_flags set")),
        [_, ..] => Err(e(UnitRule::Unit7, "a newer envelope_version")),
    }
}
