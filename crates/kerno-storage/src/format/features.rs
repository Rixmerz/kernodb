//! Feature bits (v1.md §4) and the AEAD slot policy (§5).

/// `incompat` bit 0: blocks, records and frames are sealed and the AEAD
/// slots are in use. Defined and not supported in Phase 1 (HDR-6).
pub const AEAD: u64 = 1 << 0;
/// `incompat` bit 1 (ADR-0023): the data blocks of this SST store
/// `codec: u8 ‖ body`: codec 0, `body` is the data-block payload; codec 1,
/// `body` is `raw_len: u32 LE ‖ LZ4 block` of it. The block encapsulation
/// (§7.2) is unchanged. Written on the SSTs of an engine opened with
/// `Config::block_compression`.
pub const BLOCK_LZ4: u64 = 1 << 1;
/// `incompat` bit 63: written by the pre-freeze draft and never by v1; a reader
/// refuses it by name (§4.3), it is not a known bit.
pub const DRAFT: u64 = 1 << 63;
/// `incompat` bits this reader knows (anything else is `Incompatible`).
pub const KNOWN_INCOMPAT: u64 = AEAD | BLOCK_LZ4;
/// `incompat` bits this writer sets on every file: none since the freeze.
pub const WRITE_INCOMPAT: u64 = 0;
/// `ro_compat` bits this reader knows (any other opens read-only, HDR-7).
pub const KNOWN_RO_COMPAT: u64 = 0;
/// `compat` bits this reader knows (any other is ignored, HDR-8).
pub const KNOWN_COMPAT: u64 = 0;

/// Length of the AEAD slot: `nonce[24] ‖ tag[16]`.
pub const SLOT_LEN: usize = 40;

/// Whether WAL frames always carry the AEAD slot. v1 places it only in WALs
/// whose header has `AEAD` (§5); `true` is the alternative recorded in
/// ADR-0013, "slot always present".
pub const FRAME_SLOT_ALWAYS: bool = false;

/// Whether the frames of a WAL with this `incompat` word carry a slot.
pub fn frame_has_slot(incompat: u64) -> bool {
    FRAME_SLOT_ALWAYS || incompat & AEAD != 0
}

/// Whether an AEAD slot is all zero (AEAD-1 with `AEAD` disabled).
pub fn slot_is_zero(slot: &[u8]) -> bool {
    slot.iter().all(|&b| b == 0)
}
