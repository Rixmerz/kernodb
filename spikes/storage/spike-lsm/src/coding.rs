//! Byte-level encoding helpers shared by the on-disk formats (all integers
//! little endian; variable-length integers are unsigned LEB128).

pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Varint length prefix followed by the bytes.
pub fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

pub fn u32_at(buf: &[u8], at: usize) -> Option<u32> {
    let bytes = buf.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

pub fn u64_at(buf: &[u8], at: usize) -> Option<u64> {
    let bytes = buf.get(at..at.checked_add(8)?)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

/// Bounds-checked cursor over a byte slice. Every accessor returns `None`
/// instead of panicking when the input is short or malformed, so decoders
/// can turn damaged input into `Error::Corruption`.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.pos == self.buf.len()
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }

    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    pub fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for i in 0..10 {
            let b = self.u8()?;
            if i == 9 && b > 1 {
                return None; // would overflow 64 bits
            }
            v |= u64::from(b & 0x7f) << (7 * i);
            if b < 0x80 {
                return Some(v);
            }
        }
        None
    }

    pub fn varint_usize(&mut self) -> Option<usize> {
        usize::try_from(self.varint()?).ok()
    }

    /// Varint length prefix followed by that many bytes.
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.varint_usize()?;
        self.take(n)
    }
}

/// Splits `buf` into (payload, trailing crc32c) and verifies the checksum.
pub fn verify_crc_trailer(buf: &[u8]) -> Option<&[u8]> {
    let at = buf.len().checked_sub(4)?;
    let stored = u32_at(buf, at)?;
    let payload = &buf[..at];
    (crc32c::crc32c(payload) == stored).then_some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip_and_overflow() {
        for v in [0, 1, 127, 128, 300, u32::MAX as u64, u64::MAX - 1, u64::MAX] {
            let mut b = Vec::new();
            put_varint(&mut b, v);
            let mut r = Reader::new(&b);
            assert_eq!(r.varint(), Some(v));
            assert!(r.is_empty());
        }
        // 11 continuation bytes / too many bits.
        assert_eq!(Reader::new(&[0xff; 11]).varint(), None);
        assert_eq!(Reader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]).varint(), None);
        assert_eq!(Reader::new(&[0x80]).varint(), None);
    }

    #[test]
    fn reader_is_bounds_checked() {
        let mut r = Reader::new(&[3, 1, 2]);
        assert_eq!(r.bytes(), None);
        let mut r = Reader::new(&[2, 1, 2]);
        assert_eq!(r.bytes(), Some(&[1u8, 2][..]));
        assert_eq!(r.u8(), None);
    }

    #[test]
    fn crc_trailer() {
        let mut b = b"hello".to_vec();
        let c = crc32c::crc32c(&b);
        put_u32(&mut b, c);
        assert_eq!(verify_crc_trailer(&b), Some(&b"hello"[..]));
        b[1] ^= 1;
        assert_eq!(verify_crc_trailer(&b), None);
        assert_eq!(verify_crc_trailer(&[1, 2]), None);
    }
}
