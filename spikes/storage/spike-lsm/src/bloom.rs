//! Per-SST bloom filter over user keys (~10 bits per key, 7 probes, ~1% false
//! positives). The serialized filter is `bits | k u8`; the SST wraps it in a
//! crc32c trailer, and the filter is verified when the table is opened,
//! because a damaged filter would make present keys look absent.

pub const BITS_PER_KEY: usize = 10;

/// 64-bit hash of a user key: FNV-1a followed by the murmur3 finalizer.
pub fn hash(key: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in key {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

fn probes() -> u32 {
    // k = bits_per_key * ln 2, rounded.
    ((BITS_PER_KEY as f64 * std::f64::consts::LN_2).round() as u32).clamp(1, 30)
}

/// Double hashing: probe `j` is `(h1 + j * delta) mod nbits`.
fn for_each_probe(h: u64, k: u32, nbits: u64, mut f: impl FnMut(u64) -> bool) -> bool {
    let mut a = h & 0xffff_ffff;
    let delta = (h >> 32) | 1;
    for _ in 0..k {
        if !f(a % nbits) {
            return false;
        }
        a = a.wrapping_add(delta);
    }
    true
}

/// Builds the serialized filter for the given key hashes.
pub fn build(hashes: &[u64]) -> Vec<u8> {
    let k = probes();
    let nbytes = (hashes.len() * BITS_PER_KEY).max(64).div_ceil(8);
    let nbits = (nbytes * 8) as u64;
    let mut out = vec![0u8; nbytes + 1];
    for &h in hashes {
        for_each_probe(h, k, nbits, |bit| {
            out[(bit / 8) as usize] |= 1 << (bit % 8);
            true
        });
    }
    out[nbytes] = k as u8;
    out
}

pub struct Bloom {
    bits: Vec<u8>,
    k: u32,
}

impl Bloom {
    /// Parses a serialized filter (whose checksum was already verified).
    pub fn parse(mut data: Vec<u8>) -> Option<Bloom> {
        let k = u32::from(data.pop()?);
        if data.is_empty() || !(1..=30).contains(&k) {
            return None;
        }
        Some(Bloom { bits: data, k })
    }

    pub fn may_contain(&self, h: u64) -> bool {
        let nbits = (self.bits.len() * 8) as u64;
        for_each_probe(h, self.k, nbits, |bit| self.bits[(bit / 8) as usize] & (1 << (bit % 8)) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(i: u64) -> Vec<u8> {
        format!("key-{i:08}").into_bytes()
    }

    #[test]
    fn no_false_negatives_and_low_false_positive_rate() {
        for n in [1u64, 10, 100, 10_000] {
            let hashes: Vec<u64> = (0..n).map(|i| hash(&key(i))).collect();
            let bloom = Bloom::parse(build(&hashes)).unwrap();
            for i in 0..n {
                assert!(bloom.may_contain(hash(&key(i))), "false negative for key {i} of {n}");
            }
            let probes = 20_000u64;
            let fp = (0..probes).filter(|i| bloom.may_contain(hash(&key(1_000_000 + i)))).count();
            let rate = fp as f64 / probes as f64;
            assert!(rate < 0.03, "false positive rate {rate} for {n} keys");
        }
    }

    #[test]
    fn rejects_malformed_filters() {
        assert!(Bloom::parse(vec![]).is_none());
        assert!(Bloom::parse(vec![7]).is_none());
        assert!(Bloom::parse(vec![0xff, 0]).is_none());
        assert!(Bloom::parse(vec![0xff, 31]).is_none());
    }
}
