//! Feature bits (v1.md §4) and the AEAD slot policy (§5).

/// `incompat` bit 0: blocks, records and frames are sealed and the AEAD
/// slots are in use. Defined and not supported in Phase 1 (HDR-6).
pub const AEAD: u64 = 1 << 0;
/// `incompat` bit 63: written by the pre-freeze draft (§4.3).
pub const DRAFT: u64 = 1 << 63;
/// `incompat` bits this reader knows (anything else is `Incompatible`).
pub const KNOWN_INCOMPAT: u64 = AEAD | DRAFT;
/// `incompat` bits this writer sets on every file.
pub const WRITE_INCOMPAT: u64 = DRAFT;
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
