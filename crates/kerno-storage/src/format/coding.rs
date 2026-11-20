//! Fixed-width little-endian integers, varints (VAR-1, VAR-2) and
//! length-prefixed byte strings (VAR-3), v1.md §1.1.

use super::{FormatError, Result};

pub fn put_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

pub fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Unsigned LEB128, always minimal (VAR-2).
pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Number of bytes [`put_varint`] writes for `v`.
pub fn varint_len(v: u64) -> usize {
    let bits = 64 - (v | 1).leading_zeros() as usize;
    bits.div_ceil(7)
}

/// `bytes(x) = varint(len) ‖ x`.
pub fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

/// Little-endian `u16` at `at`; the caller has checked the bounds.
pub fn le_u16(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

/// Little-endian `u32` at `at`; the caller has checked the bounds.
pub fn le_u32(buf: &[u8], at: usize) -> u32 {
    let mut b = [0; 4];
    b.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(b)
}

/// Little-endian `u64` at `at`; the caller has checked the bounds.
pub fn le_u64(buf: &[u8], at: usize) -> u64 {
    let mut b = [0; 8];
    b.copy_from_slice(&buf[at..at + 8]);
    u64::from_le_bytes(b)
}

/// Decodes one varint at the start of `buf`, returning the value and the
/// number of bytes it occupies.
///
/// * VAR-1: at most 10 bytes, the tenth `0x00` or `0x01`. A varint cut by
///   the end of `buf` is also VAR-1 here; inside a structure ([`Reader`]) it
///   is the structure's rule.
/// * VAR-2: the encoding is minimal (no final `0x00` after a continuation
///   byte).
pub fn decode_varint(buf: &[u8]) -> Result<(u64, usize)> {
    let mut v = 0u64;
    for i in 0..10 {
        let Some(&b) = buf.get(i) else {
            return Err(FormatError::corruption("VAR-1", i as u64, "truncated varint"));
        };
        if i == 9 && b > 1 {
            return Err(FormatError::corruption("VAR-1", i as u64, "varint longer than 64 bits"));
        }
        if b == 0 && i > 0 {
            return Err(FormatError::corruption("VAR-2", i as u64, "non-minimal varint"));
        }
        v |= u64::from(b & 0x7f) << (7 * i);
        if b < 0x80 {
            return Ok((v, i + 1));
        }
    }
    unreachable!("the tenth byte always ends the varint")
}

/// Bounds-checked cursor over a byte slice.
///
/// `base` is the offset of `buf[0]` reported in errors; `rule` is the rule
/// of the enclosing structure, reported when a fixed-width field is cut short
/// or bytes are left over (varints and lengths report VAR-1..3).
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    base: u64,
    rule: &'static str,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], base: u64, rule: &'static str) -> Self {
        Reader { buf, pos: 0, base, rule }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Absolute offset of the cursor (`base + pos`).
    pub fn offset(&self) -> u64 {
        self.base + self.pos as u64
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.pos == self.buf.len()
    }

    /// An error of the structure's rule at the cursor.
    pub fn error(&self, detail: &'static str) -> FormatError {
        FormatError::corruption(self.rule, self.offset(), detail)
    }

    /// An error of `rule` at the cursor.
    pub fn error_rule(&self, rule: &'static str, detail: &'static str) -> FormatError {
        FormatError::corruption(rule, self.offset(), detail)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(self.error("structure cut short"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(le_u16(self.take(2)?, 0))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(le_u32(self.take(4)?, 0))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(le_u64(self.take(8)?, 0))
    }

    /// A varint (VAR-1, VAR-2). A varint cut short by the end of the
    /// structure is the structure's rule, like any other truncated field.
    pub fn varint(&mut self) -> Result<u64> {
        let at = self.offset();
        let rest = &self.buf[self.pos..];
        match decode_varint(rest) {
            Ok((v, n)) => {
                self.pos += n;
                Ok(v)
            }
            Err(e) if e.offset as usize >= rest.len() => Err(self.error("structure cut short")),
            Err(e) => Err(e.shifted(at)),
        }
    }

    /// A varint length, checked against `limit` and the remaining bytes
    /// before anything is allocated (VAR-3).
    pub fn len(&mut self, limit: usize) -> Result<usize> {
        let at = self.offset();
        let v = self.varint()?;
        if v > limit as u64 {
            return Err(FormatError::corruption("VAR-3", at, "length above the format limit"));
        }
        if v > self.remaining() as u64 {
            return Err(FormatError::corruption("VAR-3", at, "length beyond the enclosing structure"));
        }
        Ok(v as usize)
    }

    /// `bytes(x)` with a length limit (VAR-3).
    pub fn bytes(&mut self, limit: usize) -> Result<&'a [u8]> {
        let n = self.len(limit)?;
        self.take(n)
    }

    /// Fails with the structure's rule if bytes are left over.
    pub fn finish(&self) -> Result<()> {
        if self.is_empty() { Ok(()) } else { Err(self.error("trailing bytes")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip_and_length() {
        for v in [0, 1, 127, 128, 300, 16_383, 16_384, u32::MAX as u64, u64::MAX - 1, u64::MAX] {
            let mut b = Vec::new();
            put_varint(&mut b, v);
            assert_eq!(b.len(), varint_len(v));
            assert_eq!(decode_varint(&b), Ok((v, b.len())));
        }
    }

    #[test]
    fn varint_rejects_non_minimal_and_overlong() {
        let rule = |b: &[u8]| decode_varint(b).unwrap_err().rule;
        // VAR-1: eleven bytes, a tenth byte above 1, and truncation.
        assert_eq!(rule(&[0xff; 11]), "VAR-1");
        assert_eq!(rule(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]), "VAR-1");
        assert_eq!(rule(&[0x80]), "VAR-1");
        assert_eq!(rule(&[]), "VAR-1");
        // VAR-2: a final zero after a continuation byte, at any length,
        // including a ten-byte encoding whose last byte is 0x00.
        assert_eq!(rule(&[0x80, 0x00]), "VAR-2");
        assert_eq!(rule(&[0x81, 0x80, 0x00]), "VAR-2");
        assert_eq!(rule(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]), "VAR-2");
        // The largest value is exactly ten bytes with a final 0x01.
        let max = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert_eq!(decode_varint(&max), Ok((u64::MAX, 10)));
        // A single 0x00 is the minimal encoding of 0.
        assert_eq!(decode_varint(&[0x00]), Ok((0, 1)));
    }

    #[test]
    fn reader_checks_lengths_before_allocating() {
        let mut r = Reader::new(&[0x80], 0, "X-1");
        assert_eq!(r.varint().unwrap_err().rule, "X-1");
        let mut r = Reader::new(&[3, 1, 2], 10, "X-1");
        let e = r.bytes(16).unwrap_err();
        assert_eq!((e.rule, e.offset), ("VAR-3", 10));
        let mut r = Reader::new(&[3, 1, 2, 3], 0, "X-1");
        assert_eq!(r.bytes(2).unwrap_err().rule, "VAR-3");
        let mut r = Reader::new(&[2, 1, 2, 9], 0, "X-1");
        assert_eq!(r.bytes(2).unwrap(), &[1, 2]);
        assert_eq!(r.finish().unwrap_err().rule, "X-1");
        assert_eq!(r.u16().unwrap_err().rule, "X-1");
    }
}
