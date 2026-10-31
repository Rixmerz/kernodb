//! Format limits (v1.md §1.4). Every reader enforces them (`Corruption`).
//! The Phase 1 API limits are stricter and live in `Config::limits`; raising
//! an API limit within these bounds is not a format change.

/// User key and range bound.
pub const MAX_KEY_LEN: usize = 16 << 10;
/// Value.
pub const MAX_VALUE_LEN: usize = 64 << 20;
/// Operations in one commit (`op_count`).
pub const MAX_COMMIT_OPS: u64 = 1 << 24;
/// Payload of one WAL frame (`payload_len` is 1..=this).
pub const MAX_FRAME_PAYLOAD: u32 = 1 << 30;
/// Commits in one WAL frame (`count` is 1..=this).
pub const MAX_FRAME_COMMITS: u32 = 65_536;
/// Payload of an SST data block.
pub const MAX_DATA_BLOCK_PAYLOAD: u64 = 80 << 20;
/// Payload of an SST meta block and of a MANIFEST record.
pub const MAX_META_PAYLOAD: u64 = 256 << 20;
/// `header_len` bounds (and v1's exact value).
pub const MIN_HEADER_LEN: u32 = 96;
pub const MAX_HEADER_LEN: u32 = 4096;
/// File numbers are 1..=this (DIR-1).
pub const MAX_FILE_NUMBER: u64 = (1 << 48) - 1;
/// Commit sequences are 1..=this (0 = empty database).
pub const MAX_SEQ: u64 = (1 << 63) - 1;
/// v1 accepts exactly this many levels.
pub const NUM_LEVELS: usize = 7;
/// Files per level.
pub const MAX_FILES_PER_LEVEL: usize = 1 << 20;
/// Bloom filter probes.
pub const MIN_BLOOM_K: u8 = 1;
pub const MAX_BLOOM_K: u8 = 30;
/// Bloom filter size in bits (a multiple of 8).
pub const MIN_BLOOM_BITS: u32 = 64;
pub const MAX_BLOOM_BITS: u32 = u32::MAX - 7;
/// SST `block_size` header field (informative).
pub const MIN_BLOCK_SIZE: u32 = 512;
pub const MAX_BLOCK_SIZE: u32 = 1 << 20;
