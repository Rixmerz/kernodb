//! Kerno units layer (Stage 3).
//!
//! E3-01 specifies the on-disk layout of the layer's keys and values
//! (`docs/file-format/units-v1.md`) and generates its structure vectors from
//! test helpers (`tests/`). E3-02 adds the payload integrity check
//! ([`payload`]). The rest of the codecs, the model and the API come later;
//! this crate stays separate from `kerno-storage`.

pub mod payload;

pub use payload::{
    ContentHash, HashAlg, MAX_PAYLOAD_LEN, PayloadDecl, PayloadError, PayloadRule, VerifiedPayload, verify_payload,
};
