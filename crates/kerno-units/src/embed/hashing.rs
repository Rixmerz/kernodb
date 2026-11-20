//! The deterministic provider the crate ships, for tests and wiring.

use super::{EmbedError, EmbeddingProvider};
use crate::vector_store::MAX_DIM;

/// Feature hashing of words (the "hashing trick"): each lower-cased
/// alphanumeric word adds `+1` or `-1` to one of `dim` buckets, both chosen by
/// its BLAKE3 hash; the vector is then scaled to unit length (an empty text
/// gives the zero vector). Same text, same vector, on every platform; two
/// texts that share words get a positive cosine. It knows no meaning: a real
/// model is a different [`EmbeddingProvider`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashingProvider {
    dim: usize,
}

impl HashingProvider {
    /// The `profile` it records.
    pub const PROFILE: &'static str = "test.hashing";

    /// A provider of `dim` buckets; `dim` must be in `1..=4096`.
    pub fn new(dim: usize) -> Result<Self, EmbedError> {
        if dim == 0 || dim > MAX_DIM as usize {
            return Err(EmbedError::Provider(format!("dim {dim} is outside 1..={MAX_DIM}")));
        }
        Ok(HashingProvider { dim })
    }

    fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0_f32; self.dim];
        for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
            let h = blake3::hash(word.to_lowercase().as_bytes());
            let b = h.as_bytes();
            let bucket = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) % self.dim as u64;
            v[bucket as usize] += if b[8] & 1 == 0 { 1.0 } else { -1.0 };
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        v
    }
}

impl EmbeddingProvider for HashingProvider {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }

    fn profile(&self) -> &str {
        Self::PROFILE
    }
}
