//! A deterministic stand-in for an embedding model: no model, no network.
//!
//! Every word of a text maps, through a seeded hash, to a fixed pseudo-random
//! direction; a text is the sum of the directions of its words. Two texts that
//! share words are therefore close under cosine similarity, which is all the
//! example needs. The same `(seed, dim, text)` always gives the same vector.

use std::collections::BTreeMap;

/// The stand-in "model": its seed and dimension are the recipe parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Embedder {
    pub seed: u64,
    pub dim: usize,
}

impl Default for Embedder {
    fn default() -> Self {
        Embedder { seed: 7, dim: 64 }
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3))
}

impl Embedder {
    /// The vector of `text`. Never the zero vector and always finite.
    pub fn embed(&self, text: &[u8]) -> Vec<f32> {
        let mut out = vec![0.0_f32; self.dim];
        let lower = String::from_utf8_lossy(text).to_lowercase();
        for word in lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
            let mut state = self.seed ^ fnv1a(word.as_bytes());
            for slot in &mut out {
                // 24 random bits mapped to [-1, 1).
                let bits = (splitmix64(&mut state) >> 40) as f32;
                *slot += bits / (1u32 << 23) as f32 - 1.0;
            }
        }
        if out.iter().all(|x| *x == 0.0) {
            out[0] = 1.0;
        }
        out
    }

    /// The recipe parameters recorded with every vector this model makes.
    pub fn params(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("dim".to_owned(), self.dim.to_string()), ("seed".to_owned(), self.seed.to_string())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_seeded() {
        let a = Embedder { seed: 1, dim: 8 };
        assert_eq!(a.embed(b"hello world"), a.embed(b"hello world"));
        assert_ne!(a.embed(b"hello world"), Embedder { seed: 2, dim: 8 }.embed(b"hello world"));
        assert_eq!(a.embed(b"").len(), 8);
    }
}
