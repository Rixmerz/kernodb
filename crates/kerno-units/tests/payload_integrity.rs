//! E3-02: payload integrity (`units-v1.md` §5.1). Known-answer hash vectors,
//! truncation, corruption, limits and rule order.

use kerno_units::payload::PAYLOAD_CODEC_OPAQUE;
use kerno_units::{ContentHash, HashAlg, MAX_PAYLOAD_LEN, PayloadDecl, PayloadRule, verify_payload};
use proptest::prelude::*;

fn hex(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// Input of the official BLAKE3 test vectors: byte `i` is `i % 251`.
fn official_input(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn decl_for(payload: &[u8], alg: HashAlg) -> PayloadDecl {
    PayloadDecl {
        content_hash: ContentHash::compute(alg, PAYLOAD_CODEC_OPAQUE, payload),
        payload_len: payload.len() as u32,
        payload_codec: PAYLOAD_CODEC_OPAQUE,
        payload_present: true,
    }
}

fn rule(decl: &PayloadDecl, stored: Option<&[u8]>) -> Option<PayloadRule> {
    verify_payload(decl, stored).err().map(|e| e.rule)
}

// ---- known answers -------------------------------------------------------

/// The hash input is `codec ‖ payload` with codec 0, so the content hash of
/// `official_input(n)[1..]` is the official BLAKE3 vector for length `n`.
#[test]
fn blake3_known_answers() {
    let cases = [
        (1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"),
        (2, "7b7015bb92cf0b318037702a6cdd81dee41224f734684c2c122cd6359cb1ee63"),
        (1024, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"),
    ];
    for (n, want) in cases {
        let input = official_input(n);
        let h = ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, &input[1..]);
        assert_eq!(h.alg, HashAlg::Blake3);
        assert_eq!(h.digest, hex(want), "official BLAKE3 vector, length {n}");
    }
}

#[test]
fn crc32c_known_answer() {
    // CRC-32C("123456789") = 0xE3069283 (RFC 3720 check value). The hash
    // input is codec ‖ payload, so codec b'1' and payload "23456789" hash
    // exactly that string.
    let h = ContentHash::compute(HashAlg::Crc32cProvisional, b'1', b"23456789");
    assert_eq!(&h.digest[..4], &0xE306_9283u32.to_be_bytes());
    assert!(h.digest[4..].iter().all(|&b| b == 0), "UNIT-14 padding");
}

// ---- accepted ------------------------------------------------------------

#[test]
fn valid_payloads_are_accepted() {
    for alg in [HashAlg::Blake3, HashAlg::Crc32cProvisional] {
        for p in [&b""[..], b"x", b"hello units"] {
            let d = decl_for(p, alg);
            assert_eq!(verify_payload(&d, Some(p)).unwrap().bytes(), Some(p));
        }
    }
    let absent = PayloadDecl {
        content_hash: ContentHash::blake3(0, b""),
        payload_len: 0,
        payload_codec: 0,
        payload_present: false,
    };
    assert_eq!(verify_payload(&absent, None).unwrap().bytes(), None);
}

#[test]
fn maximum_length_is_accepted_and_one_more_is_not() {
    let p = vec![0xA5u8; MAX_PAYLOAD_LEN as usize];
    assert!(verify_payload(&decl_for(&p, HashAlg::Blake3), Some(&p)).is_ok());
    let mut d = decl_for(&p, HashAlg::Blake3);
    d.payload_len = MAX_PAYLOAD_LEN + 1;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit22));
    d.payload_len = u32::MAX;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit22));
}

// ---- rejected ------------------------------------------------------------

#[test]
fn every_truncation_is_unit27() {
    let p = official_input(300);
    let d = decl_for(&p, HashAlg::Blake3);
    for cut in 0..p.len() {
        assert_eq!(rule(&d, Some(&p[..cut])), Some(PayloadRule::Unit27), "cut {cut}");
    }
}

#[test]
fn extension_is_unit27() {
    let p = official_input(64);
    let d = decl_for(&p, HashAlg::Blake3);
    let mut longer = p.clone();
    longer.push(0);
    assert_eq!(rule(&d, Some(&longer)), Some(PayloadRule::Unit27));
}

#[test]
fn declared_length_mismatch_is_unit27() {
    let p = official_input(64);
    for len in [0, 63, 65, MAX_PAYLOAD_LEN] {
        let mut d = decl_for(&p, HashAlg::Blake3);
        d.payload_len = len;
        assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit27), "len {len}");
    }
}

#[test]
fn every_payload_bit_flip_is_unit28() {
    for alg in [HashAlg::Blake3, HashAlg::Crc32cProvisional] {
        let p = official_input(64);
        let d = decl_for(&p, alg);
        for bit in 0..p.len() * 8 {
            let mut q = p.clone();
            q[bit / 8] ^= 1 << (bit % 8);
            assert_eq!(rule(&d, Some(&q)), Some(PayloadRule::Unit28), "{alg:?} bit {bit}");
        }
    }
}

#[test]
fn every_digest_bit_flip_is_unit28() {
    for alg in [HashAlg::Blake3, HashAlg::Crc32cProvisional] {
        let p = b"payload".to_vec();
        for bit in 0..256 {
            let mut d = decl_for(&p, alg);
            d.content_hash.digest[bit / 8] ^= 1 << (bit % 8);
            assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit28), "{alg:?} bit {bit}");
        }
    }
}

#[test]
fn wrong_algorithm_is_unit28() {
    let p = b"payload".to_vec();
    let mut d = decl_for(&p, HashAlg::Blake3);
    d.content_hash.alg = HashAlg::Crc32cProvisional;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit28));
}

#[test]
fn presence_mismatch_is_unit26() {
    let p = b"abc".to_vec();
    let present = decl_for(&p, HashAlg::Blake3);
    assert_eq!(rule(&present, None), Some(PayloadRule::Unit26));
    let mut absent = present;
    absent.payload_present = false;
    assert_eq!(rule(&absent, Some(&p)), Some(PayloadRule::Unit26));
    assert_eq!(rule(&absent, None), Some(PayloadRule::Unit26), "len 3 without payload");
    absent.payload_len = 0;
    assert_eq!(rule(&absent, Some(b"")), Some(PayloadRule::Unit26));
}

#[test]
fn codec_other_than_zero_is_unit13() {
    let p = b"abc".to_vec();
    let mut d = decl_for(&p, HashAlg::Blake3);
    d.payload_codec = 1;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit13Codec));
}

#[test]
fn rules_apply_in_order() {
    let p = b"abc".to_vec();
    let mut d = decl_for(&p, HashAlg::Blake3);
    d.content_hash.digest[0] ^= 1;
    d.payload_len = 2;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit27), "length before hash");
    d.payload_present = false;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit26), "presence before length");
    d.payload_len = MAX_PAYLOAD_LEN + 1;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit22), "limit before presence");
    d.payload_codec = 7;
    assert_eq!(rule(&d, Some(&p)), Some(PayloadRule::Unit13Codec), "codec first");
}

#[test]
fn errors_are_deterministic_corruption() {
    let p = official_input(32);
    let d = decl_for(&p, HashAlg::Blake3);
    let e1 = verify_payload(&d, Some(&p[..10])).unwrap_err();
    let e2 = verify_payload(&d, Some(&p[..10])).unwrap_err();
    assert_eq!(e1, e2);
    assert!(e1.to_string().starts_with("UNIT-27: "));
    let core: kerno_core::Error = e1.into();
    assert!(matches!(core, kerno_core::Error::Corruption(_)));
    // The rejection left nothing behind: the valid bytes still verify.
    assert!(verify_payload(&d, Some(&p)).is_ok());
}

proptest! {
    /// Whatever the stored bytes are, they are accepted only if they equal
    /// the bytes the declaration was made from.
    #[test]
    fn only_the_declared_bytes_are_accepted(
        original in proptest::collection::vec(any::<u8>(), 0..512),
        stored in proptest::collection::vec(any::<u8>(), 0..512),
        crc in any::<bool>(),
    ) {
        let alg = if crc { HashAlg::Crc32cProvisional } else { HashAlg::Blake3 };
        let d = decl_for(&original, alg);
        let ok = verify_payload(&d, Some(&stored)).is_ok();
        if stored == original {
            prop_assert!(ok);
        } else if alg == HashAlg::Blake3 {
            prop_assert!(!ok);
        }
    }

    /// Any single mutation (truncate, extend, flip) of a BLAKE3 payload is rejected.
    #[test]
    fn single_mutation_is_rejected(
        original in proptest::collection::vec(any::<u8>(), 1..512),
        pos in any::<prop::sample::Index>(),
        kind in 0u8..3,
        bit in 0u8..8,
    ) {
        let d = decl_for(&original, HashAlg::Blake3);
        let i = pos.index(original.len());
        let mut q = original.clone();
        let want = match kind {
            0 => { q.truncate(i); PayloadRule::Unit27 }
            1 => { q.insert(i, bit); PayloadRule::Unit27 }
            _ => { q[i] ^= 1 << bit; PayloadRule::Unit28 }
        };
        prop_assert_eq!(rule(&d, Some(&q)), Some(want));
    }
}
