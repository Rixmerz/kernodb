//! Kerno units layer (Stage 3).
//!
//! E3-01 only specifies the on-disk layout of the layer's keys and values
//! (`docs/file-format/units-v1.md`) and generates its structure vectors from
//! test helpers (`tests/`); the production codecs, the model and the API are
//! E3-02 onwards, and this crate stays separate from `kerno-storage`.
