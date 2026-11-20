//! TLV fields (v1.md §1.3): `tag varint ‖ len varint ‖ value[len]`.
//!
//! Ordering, repetition and unknown tags depend on the structure (SST
//! properties §7.6, MANIFEST fields §8.3); this module only frames the
//! fields and decodes the two scalar value types.

use super::coding::{Reader, decode_varint, put_varint};
use super::{FormatError, Result};

/// Tags from this value up are ignored by readers that do not know them.
pub const FIRST_IGNORABLE_TAG: u64 = 1024;

pub fn put_tlv(out: &mut Vec<u8>, tag: u64, value: &[u8]) {
    put_varint(out, tag);
    put_varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

pub fn put_tlv_varint(out: &mut Vec<u8>, tag: u64, v: u64) {
    let mut value = Vec::with_capacity(10);
    put_varint(&mut value, v);
    put_tlv(out, tag, &value);
}

pub fn put_tlv_u8(out: &mut Vec<u8>, tag: u64, v: u8) {
    put_tlv(out, tag, &[v]);
}

/// One decoded field; `offset` is the absolute offset of its tag and
/// `value_offset` that of its value.
#[derive(Clone, Copy, Debug)]
pub struct Tlv<'a> {
    pub tag: u64,
    pub value: &'a [u8],
    pub offset: u64,
    pub value_offset: u64,
}

/// Iterates over the fields of `buf`. A length beyond the end of `buf` is
/// VAR-3; malformed varints are VAR-1/VAR-2.
pub struct TlvIter<'a> {
    reader: Reader<'a>,
    failed: bool,
}

impl<'a> TlvIter<'a> {
    pub fn new(buf: &'a [u8], base: u64, rule: &'static str) -> Self {
        TlvIter { reader: Reader::new(buf, base, rule), failed: false }
    }
}

impl<'a> Iterator for TlvIter<'a> {
    type Item = Result<Tlv<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.is_empty() {
            return None;
        }
        let offset = self.reader.offset();
        let field = (|| {
            let tag = self.reader.varint()?;
            let n = self.reader.len(usize::MAX)?;
            let value_offset = self.reader.offset();
            let value = self.reader.take(n)?;
            Ok(Tlv { tag, value, offset, value_offset })
        })();
        self.failed = field.is_err();
        Some(field)
    }
}

/// A varint value, which must occupy exactly the field's `len` bytes: a
/// varint cut by the end of the value, or followed by more bytes, violates
/// the structure's `rule`; VAR-1 and VAR-2 inside the value are reported as
/// such.
pub fn value_varint(field: &Tlv<'_>, rule: &'static str) -> Result<u64> {
    let at = field.value_offset;
    match decode_varint(field.value) {
        Ok((v, n)) if n == field.value.len() => Ok(v),
        Ok(_) => Err(FormatError::corruption(rule, at, "varint field with trailing bytes")),
        Err(e) if e.offset as usize >= field.value.len() => {
            Err(FormatError::corruption(rule, at, "varint field longer than its len"))
        }
        Err(e) => Err(e.shifted(at)),
    }
}

/// A `u8` value: `len = 1`.
pub fn value_u8(field: &Tlv<'_>, rule: &'static str) -> Result<u8> {
    match field.value {
        [v] => Ok(*v),
        _ => Err(FormatError::corruption(rule, field.value_offset, "u8 field with len != 1")),
    }
}
