//! `incompat` bit `BLOCK_LZ4` (v1.md §4.4, ADR-0023): SST data blocks stored as
//! `codec ‖ body`. The block encapsulation is unchanged, so every other rule of v1 applies.

mod spec_decoder;

use std::path::Path;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::StdVfs;
use kerno_storage::format::block::{self, Entry, reseal_block};
use kerno_storage::format::features::BLOCK_LZ4;
use kerno_storage::format::sst::{SstBuilder, SstExpect, open_sst, read_sst};
use kerno_storage::{Config, KernoDb, VerifyOptions, verify};

const DB: [u8; 16] = [7; 16];
const FILE: u64 = 9;

/// A deterministic byte stream: incompressible.
fn noise(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// JSON-like values: compressible, as most application data.
fn json(i: usize) -> Vec<u8> {
    format!(r#"{{"qname":"crates.kerno.src.module_{i}.function_{i}","kind":"function","file":"src/m{i}.rs"}}"#)
        .into_bytes()
}

/// `(key, value)` pairs in key order.
type Pairs = Vec<(Vec<u8>, Vec<u8>)>;

fn table(lz4: bool, value: impl Fn(usize) -> Vec<u8>) -> (Vec<u8>, Pairs) {
    let mut b = SstBuilder::with_compression(DB, FILE, 4096, lz4);
    let mut want = Vec::new();
    for i in 0..600 {
        let (k, v) = (format!("key-{i:05}").into_bytes(), value(i));
        b.add(&k, i as u64 + 1, Some(&v));
        want.push((k, v));
    }
    let (file, _, _) = b.finish::<Vec<u8>>(&[], kerno_storage::format::props::REASON_FLUSH);
    (file, want)
}

fn expect() -> SstExpect<'static> {
    SstExpect { file_number: FILE, db_id: DB, meta: None, last_seq: 10_000 }
}

fn spec(file: &[u8]) -> Result<String, spec_decoder::Fail> {
    spec_decoder::decode(&spec_decoder::Ctx::Sst { file_number: FILE, db_id: DB, meta: None, last_seq: 10_000 }, file)
}

fn incompat(file: &[u8]) -> u64 {
    u64::from_le_bytes(file[32..40].try_into().unwrap())
}

#[test]
fn compressed_and_plain_tables_hold_the_same_entries_and_both_decoders_agree() {
    for (name, value) in [("json", &json as &dyn Fn(usize) -> Vec<u8>), ("noise", &|i| noise(i as u64, 300))] {
        let (plain, want) = table(false, value);
        let (packed, _) = table(true, value);
        assert_eq!(incompat(&plain) & BLOCK_LZ4, 0, "{name}: a plain table has no bit");
        assert_ne!(incompat(&packed) & BLOCK_LZ4, 0, "{name}: a compressed table has the bit");
        for file in [&plain, &packed] {
            let (_, entries) = read_sst(file, &expect()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let got: Vec<_> = entries.into_iter().map(|e: Entry| (e.key, e.value.unwrap())).collect();
            assert_eq!(got, want, "{name}");
            spec(file).unwrap_or_else(|e| panic!("{name}: the spec decoder refuses it: {e:?}"));
        }
        if name == "json" {
            assert!(packed.len() * 2 < plain.len(), "json compresses: {} vs {}", packed.len(), plain.len());
        } else {
            // Incompressible blocks are stored raw: one codec byte per block more, no worse.
            assert!(packed.len() <= plain.len() + 64, "noise: {} vs {}", packed.len(), plain.len());
        }
    }
}

/// The close-time flush skips the LZ4 pass: the table keeps the bit and stores every block as is
/// (codec 0), so it is larger than a compressed one, and both decoders read it.
#[test]
fn a_table_without_the_lz4_pass_keeps_the_bit_stores_codec_0_and_both_decoders_agree() {
    let build = |skip: bool| {
        let mut b = SstBuilder::with_compression(DB, FILE, 4096, true);
        if skip {
            b = b.without_lz4_pass();
        }
        let mut want = Vec::new();
        for i in 0..600 {
            let (k, v) = (format!("key-{i:05}").into_bytes(), json(i));
            b.add(&k, i as u64 + 1, Some(&v));
            want.push((k, v));
        }
        let (file, _, _) = b.finish::<Vec<u8>>(&[], kerno_storage::format::props::REASON_FLUSH);
        (file, want)
    };
    let (packed, want) = build(false);
    let (raw, _) = build(true);
    assert_ne!(incompat(&raw) & BLOCK_LZ4, 0, "the table still has the bit");
    assert!(raw.len() > packed.len() * 2, "nothing was compressed: {} vs {}", raw.len(), packed.len());
    let (_, entries) = read_sst(&raw, &expect()).unwrap();
    let got: Vec<_> = entries.into_iter().map(|e: Entry| (e.key, e.value.unwrap())).collect();
    assert_eq!(got, want);
    spec(&raw).unwrap_or_else(|e| panic!("the spec decoder refuses it: {e:?}"));
}

/// The first data block of `file` replaced by `stored` (its payload), resealed so the CRC
/// passes and only LZ4-1 can find the damage. Same length keeps every offset valid.
fn with_first_payload(file: &[u8], f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let meta = open_sst(file, &expect()).unwrap();
    let h = meta.index[0].handle;
    let (start, len) = (h.offset as usize, h.len as usize);
    let trailer = block::BLOCK_TRAILER_LEN;
    let mut payload = file[start..start + len - trailer].to_vec();
    f(&mut payload);
    assert_eq!(payload.len(), len - trailer, "the patch keeps the length");
    let mut out = file.to_vec();
    out[start..start + len - trailer].copy_from_slice(&payload);
    reseal_block(&mut out[start..start + len], FILE, h.offset);
    out
}

#[test]
fn lz4_1_unknown_codec_bad_length_or_stream_is_sst_r1_for_both_decoders() {
    let (packed, _) = table(true, json);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("unknown codec", with_first_payload(&packed, |p| p[0] = 2)),
        ("length above the limit", with_first_payload(&packed, |p| p[1..5].copy_from_slice(&u32::MAX.to_le_bytes()))),
        ("length that does not match", with_first_payload(&packed, |p| p[1] ^= 1)),
        (
            "damaged stream",
            with_first_payload(&packed, |p| {
                let n = p.len();
                p[n - 3] = 0xFF;
                p[n - 2] = 0xFF;
            }),
        ),
    ];
    for (name, file) in cases {
        let e = read_sst(&file, &expect()).expect_err(name);
        assert_eq!(e.rule, "SST-R1", "{name}: {e:?}");
        assert_eq!(spec(&file).map(|_| ()).unwrap_err().0, "SST-R1", "{name}");
    }
}

#[test]
fn the_bit_is_refused_outside_an_sst() {
    use kerno_storage::format::header::{FileKind, HeaderExpect, decode_header, reseal_header};
    let (packed, _) = table(true, json);
    // The SST's header turned into a WAL header that keeps the bit, with a valid CRC: only
    // HDR-6 can refuse it (§4.4: the bit has a meaning only in an SST).
    let mut h = packed[..96].to_vec();
    h[0..8].copy_from_slice(b"KRNO_WAL");
    h[64..72].copy_from_slice(&1u64.to_le_bytes()); // base_seq
    reseal_header(&mut h);
    let e = decode_header(&h, 96, FileKind::Wal, &HeaderExpect::default()).unwrap_err();
    assert_eq!(e.rule, "HDR-6");
    let ctx = spec_decoder::Ctx::Header { kind: spec_decoder::FileType::Wal, db_id: None, file_number: None };
    assert_eq!(spec_decoder::decode(&ctx, &h).unwrap_err().0, "HDR-6");
}

fn engine(dir: &Path, compress: bool) -> KernoDb {
    let cfg = Config { block_compression: compress, allow_unverified_platform: true, ..Config::default() };
    KernoDb::open(StdVfs::shared(), dir, &cfg).unwrap()
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum()
}

#[test]
fn an_engine_with_compression_reads_back_verifies_and_is_smaller() {
    let root = std::env::temp_dir().join(format!("kerno-lz4-{}", std::process::id()));
    let mut sizes = Vec::new();
    for compress in [false, true] {
        let dir = root.join(if compress { "lz4" } else { "plain" });
        let db = engine(&dir, compress);
        for chunk in 0..20 {
            let mut b = WriteBatch::new();
            for i in chunk * 500..(chunk + 1) * 500 {
                b.put(format!("key-{i:06}"), json(i));
            }
            db.commit(&b).unwrap();
        }
        db.hooks().flush().unwrap();
        db.close().unwrap();
        // Reopen: every value reads back, and a deep verify is clean.
        let db = engine(&dir, compress);
        let snap = db.snapshot().unwrap();
        for i in (0..10_000).step_by(97) {
            assert_eq!(snap.get(format!("key-{i:06}").as_bytes()).unwrap().as_deref(), Some(&json(i)[..]));
        }
        drop(snap);
        db.close().unwrap();
        let report = verify(StdVfs::shared(), &dir, &VerifyOptions { deep: true, check_wal: true }).unwrap();
        assert!(report.issues.is_empty(), "compress={compress}: {:?}", report.issues);
        let ssts: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "sst"))
            .collect();
        assert!(!ssts.is_empty());
        for p in &ssts {
            let bit = incompat(&std::fs::read(p).unwrap()) & BLOCK_LZ4 != 0;
            assert_eq!(bit, compress, "{p:?}");
        }
        sizes.push(dir_bytes(&dir));
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(sizes[1] * 2 < sizes[0], "compressed {} vs plain {}", sizes[1], sizes[0]);
}
