//! CRC-32C and context CRCs (v1.md §1.2).
//!
//! `crc(ctx; data) = crc32c(ctx ‖ data)`, where the reader supplies `ctx`
//! (file number, block offset, MANIFEST generation): a block copied from
//! another file or offset does not verify.

/// Plain CRC-32C (Castagnoli, the `crc32c` crate).
pub fn crc32c(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
}

/// `crc32c(ctx ‖ data)`.
pub fn crc_ctx(ctx: &[u8], data: &[u8]) -> u32 {
    crc32c::crc32c_append(crc32c::crc32c(ctx), data)
}

/// `crc32c(ctx ‖ parts[0] ‖ parts[1] ‖ ...)`.
pub fn crc_ctx_parts(ctx: &[u8], parts: &[&[u8]]) -> u32 {
    parts.iter().fold(crc32c::crc32c(ctx), |crc, p| crc32c::crc32c_append(crc, p))
}

/// Context of WAL frames and of the SST footer: `file_number u64`. Also the
/// context of MANIFEST records, with the generation as the number.
pub fn file_ctx(file_number: u64) -> [u8; 8] {
    file_number.to_le_bytes()
}

/// Context of SST blocks: `file_number u64 ‖ block_offset u64`.
pub fn block_ctx(file_number: u64, block_offset: u64) -> [u8; 16] {
    let mut c = [0; 16];
    c[..8].copy_from_slice(&file_number.to_le_bytes());
    c[8..].copy_from_slice(&block_offset.to_le_bytes());
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn context_crc_is_crc_of_concatenation() {
        let ctx = block_ctx(7, 4096);
        let mut all = ctx.to_vec();
        all.extend_from_slice(b"payload");
        all.extend_from_slice(&[0; 40]);
        assert_eq!(crc_ctx_parts(&ctx, &[b"payload", &[0; 40]]), crc32c(&all));
        assert_eq!(crc_ctx(&file_ctx(1), b"x"), crc32c(&[1, 0, 0, 0, 0, 0, 0, 0, b'x']));
        assert_ne!(crc_ctx(&file_ctx(1), b"x"), crc_ctx(&file_ctx(2), b"x"));
    }
}
