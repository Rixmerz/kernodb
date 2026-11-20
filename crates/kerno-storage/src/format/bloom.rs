//! Bloom filter hash and probes, frozen in v1.md §12. They are the function
//! of the M0 prototype (`spikes/storage/spike-lsm/src/bloom.rs`).

/// Probes v1 writes (`k`).
pub const WRITER_K: u8 = 7;
/// Bits per key v1 writes.
pub const BITS_PER_KEY: u64 = 10;

/// `fmix64(fnv1a64(key))`.
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

/// Calls `f` with the bit index of probe `j` for `j` in `0..k`:
/// `a_0 = h & 0xffffffff`, `a_{j+1} = a_j + ((h >> 32) | 1)` (mod 2^64),
/// bit `a_j mod nbits`. Stops early when `f` returns `false`.
pub fn for_each_probe(h: u64, k: u8, nbits: u64, mut f: impl FnMut(u64) -> bool) -> bool {
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

/// Sets the probes of `h` in `bits` (bit `i` is `bits[i / 8] & (1 << (i % 8))`).
pub fn insert(bits: &mut [u8], k: u8, h: u64) {
    let nbits = bits.len() as u64 * 8;
    for_each_probe(h, k, nbits, |i| {
        bits[(i / 8) as usize] |= 1 << (i % 8);
        true
    });
}

/// Whether every probe of `h` is set.
pub fn may_contain(bits: &[u8], k: u8, h: u64) -> bool {
    let nbits = bits.len() as u64 * 8;
    nbits > 0 && for_each_probe(h, k, nbits, |i| bits[(i / 8) as usize] & (1 << (i % 8)) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_testkit::rng::Rng;

    /// Verbatim copy of the M0 functions (`spike-lsm/src/bloom.rs` at
    /// `0cba925`): the oracle for §12.
    mod m0 {
        pub const BITS_PER_KEY: usize = 10;

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

        pub fn probes() -> u32 {
            ((BITS_PER_KEY as f64 * std::f64::consts::LN_2).round() as u32).clamp(1, 30)
        }

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
    }

    #[test]
    fn bloom_matches_m0_function() {
        assert_eq!(u32::from(WRITER_K), m0::probes());
        assert_eq!(BITS_PER_KEY as usize, m0::BITS_PER_KEY);
        let mut rng = Rng::new(12);
        for n in [0usize, 1, 2, 7, 100, 1000] {
            let keys: Vec<Vec<u8>> = (0..n)
                .map(|_| {
                    let len = rng.below(24) as usize;
                    rng.bytes(len)
                })
                .collect();
            let hashes: Vec<u64> = keys.iter().map(|k| hash(k)).collect();
            assert_eq!(hashes, keys.iter().map(|k| m0::hash(k)).collect::<Vec<_>>());
            // Same bits as the M0 serialization (`bits ‖ k`), with v1's size rule.
            let filter = crate::format::filter::Filter::build(&hashes);
            let mut m0 = m0::build(&hashes);
            let k = m0.pop().unwrap();
            assert_eq!((filter.k, &filter.bits), (k, &m0));
            for h in &hashes {
                assert!(may_contain(&filter.bits, filter.k, *h));
            }
        }
    }
}
