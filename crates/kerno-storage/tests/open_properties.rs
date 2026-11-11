//! Open properties under hostile files (E9A-03, docs/architecture/
//! etapa9-adelanto-acta.md §3.2 and §5).
//!
//! A database is created once (flushed and compacted tables, a live WAL with
//! unflushed commits, no clean close). Each case takes a seed, derives from it
//! one mutation of one file of one type (WAL, SST or MANIFEST), then opens the
//! mutated directory read-write and read-only and calls `verify()`. The
//! mutation is a bit flip, a truncation, or a rewrite of a few bytes of a
//! structural field with the CRC of the unit recomputed (so the checksum
//! passes and the structural rules are what must catch it).
//!
//! Properties, all of them for every case:
//!
//! * no panic;
//! * the only errors are `Corruption` and `Incompatible` (from `verify`,
//!   `open`, `snapshot`, `get` and `scan`);
//! * an open that succeeds shows exactly the model at the sequence it
//!   reports (`models[seq]`): the final state, or an earlier state of the
//!   history when the damage looks like a torn log tail. Never a mixture and
//!   never a value that was not written. One exception, for a rewritten
//!   filter block only: `get` trusts the filter and a CRC cannot tell a
//!   rewritten filter from the real one, so a `get` may disagree with the
//!   scan, but only when `verify()` reports the directory (see
//!   `known_limit_get_trusts_a_resealed_filter_missing_interior_keys`);
//! * `verify()` and the read-only open leave the directory unchanged.
//!
//! Threshold (acta §5): 2 000 cases per file type and per mode, overridable
//! with `PROPTEST_CASES`; each case has its own seed, printed on failure, and
//! `failure_persistence` is off so that no regression file is written.
//! Re-run one case with `OPEN_PROPERTIES_SEED=<seed>` (all six properties
//! then run only that seed).
//!
//! Limit of the rewrite: a CRC cannot tell a rewritten user value from the
//! real one, so rewrites with a recomputed CRC target only structure: the
//! common headers, the footer, the meta blocks (the filter among them), the
//! offset array and count of a data block, the header and first payload bytes of a WAL frame, and the
//! MANIFEST records. A rewrite of a value byte would make the "no different
//! data" property unsatisfiable by design.
//!
//! Mutation record (acta §3.2), run on a copy of the tree: `open_block`
//! (src/format/block.rs) with its CRC comparison replaced by `false && ...`.
//! Result: `sst_read_write`, `sst_read_only` and
//! `mutations_reach_both_outcomes_and_change_the_file` fail ("the scan at
//! sequence 409 is not the model: wrong value for key-00096", seed
//! 1618401364763173671); the other six tests stay green. Nothing was mutated in
//! the working tree.
//!
//! Mutation record of the fix for F-E9A-03-1, each applied in place, run and
//! reverted by hand (`git diff` shows none left):
//!
//! * `check_filter_holds_index_keys` called in `Table::open_file`
//!   (src/sst.rs) with its result dropped: `resealed_zeroed_filter_is_refused_
//!   at_open_and_reported_by_verify` fails ("a table whose filter lost every
//!   bit opened"); the `get` limit test stays green.
//! * The same call in `verify.rs` disabled: the same test fails
//!   (`deep=true: one SST-V3 per table, at its filter block`: the issues fall
//!   back to the data block at offset 96).
//! * The SST-V3 probe of `SstVerifier::observe` (src/format/sst.rs) disabled:
//!   `known_limit_get_trusts_...` fails (deep verify reports nothing), and so
//!   do `sst_read_write`, `sst_read_only` and
//!   `mutations_reach_both_outcomes_and_change_the_file` ("get() disagrees
//!   with the scan and verify() reported nothing", e.g. a one-byte rewrite
//!   of the filter of 000013.sst): the filter rewrites do reach silent hiding.
//! * `Table::get` made to ignore the filter: `known_limit_get_trusts_...`
//!   fails ("the limit is gone"); everything else stays green.
//! * `case` passing `tolerate_get = false`: the same three SST tests fail
//!   with "get(..) returned wrong data: None".
//!
//! Finding F-E9A-03-1 (fixed, docs/security/threat-model.md §6.6): see
//! `resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` and
//! `known_limit_get_trusts_a_resealed_filter_missing_interior_keys`.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::format::block::{BLOCK_TRAILER_LEN, Entry, open_block, reseal_block};
use kerno_storage::format::bloom;
use kerno_storage::format::filter::FILTER_HEADER_LEN;
use kerno_storage::format::header::{HEADER_LEN, reseal_header};
use kerno_storage::format::manifest::{RECORD_HEADER_LEN, reseal_record};
use kerno_storage::format::sst::{
    FOOTER_LEN, SstExpect, SstMeta, decode_footer, decode_index, read_sst, reseal_footer,
};
use kerno_storage::format::wal::{FRAME_HEADER_LEN, reseal_frame};
use kerno_storage::{AccessMode, Background, CloseBudget, Config, KernoDb, TestConfig, VerifyOptions, verify};
use kerno_testkit::harness::{Model, apply, diff, read_all};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{DirImage, SimVfs, assert_dir_unchanged};
use proptest::prelude::*;

const DIR: &str = "/db";
const DB_ID: [u8; 16] = [0xd1; 16];
/// AEAD slot length of a block or record (no encryption: all zero).
const SLOT_LEN: usize = 40;

// ---- the database under test (helpers copied from tests/common) --------------

fn config(access: AccessMode) -> Config {
    Config {
        durability: Durability::Sync,
        write_buffer_bytes: 16 << 10,
        cache_bytes: 1 << 20,
        access,
        test: TestConfig { background: Background::Inline, db_id: Some(DB_ID) },
        close_budget: CloseBudget::Unbounded,
        ..Config::default()
    }
}

fn open_in(vfs: &Arc<dyn Vfs>, access: AccessMode) -> kerno_core::Result<KernoDb> {
    KernoDb::open(vfs.clone(), Path::new(DIR), &config(access))
}

/// The base image, and `models[s]`: the state at sequence `s`.
struct Base {
    image: DirImage,
    models: Vec<Model>,
}

fn base() -> &'static Base {
    static BASE: OnceLock<Base> = OnceLock::new();
    BASE.get_or_init(|| {
        let disk = SimVfs::new(2);
        let db = open_in(&disk.as_vfs(), AccessMode::ReadWrite).unwrap();
        let mut models = vec![Model::new()];
        let mut commit = |b: WriteBatch| {
            let mut next = models.last().unwrap().clone();
            apply(&mut next, &b);
            assert_eq!(db.commit(&b).unwrap(), models.len() as u64);
            models.push(next);
        };
        for i in 0..400u32 {
            let mut b = WriteBatch::new();
            b.put(format!("key-{i:05}"), vec![i as u8; 200]);
            if i % 9 == 8 {
                b.delete(format!("key-{:05}", i - 5));
            }
            commit(b);
        }
        // A live tail the engine has not flushed.
        for i in 400..408u32 {
            let mut b = WriteBatch::new();
            b.put(format!("key-{i:05}"), vec![i as u8; 200]);
            commit(b);
        }
        let mut b = WriteBatch::new();
        b.delete_range("key-00300", "key-00310");
        b.put("key-00305", "again");
        commit(b);
        drop(db); // no clean close
        let image = disk.dump();
        assert!(files_with(&image, "sst").len() >= 3, "the base needs several tables");
        assert!(!files_with(&image, "wal").is_empty());
        Base { image, models }
    })
}

/// The files of `image` with this extension, or the MANIFEST, by name.
fn files_with(image: &DirImage, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = image
        .files
        .keys()
        .filter(|p| match ext {
            "MANIFEST" => p.file_name().is_some_and(|n| n == "MANIFEST"),
            ext => p.extension().is_some_and(|e| e == ext),
        })
        .cloned()
        .collect();
    v.sort();
    v
}

// ---- mutations ---------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum FileType {
    Wal,
    Sst,
    Manifest,
}

impl FileType {
    fn ext(self) -> &'static str {
        match self {
            FileType::Wal => "wal",
            FileType::Sst => "sst",
            FileType::Manifest => "MANIFEST",
        }
    }
}

/// How the CRC of a unit is recomputed after a rewrite.
#[derive(Clone, Copy, Debug)]
enum Seal {
    Header,
    Block,
    Footer,
    Frame,
    Record { generation: u64 },
}

/// A unit with its own checksum: `start..end` are its bytes, `targets` the
/// bytes a rewrite may change and `skip` the CRC field inside them.
#[derive(Clone, Debug)]
struct Unit {
    start: usize,
    end: usize,
    targets: Range<usize>,
    skip: Option<Range<usize>>,
    seal: Seal,
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn le_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn file_number(p: &Path) -> u64 {
    p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn header_unit() -> Unit {
    Unit { start: 0, end: HEADER_LEN, targets: 0..HEADER_LEN - 4, skip: None, seal: Seal::Header }
}

/// The checksummed units of `bytes`, found through the unmutated structure.
fn units(ty: FileType, name: &Path, bytes: &[u8]) -> Vec<Unit> {
    let mut out = vec![header_unit()];
    if bytes.len() < HEADER_LEN {
        return out;
    }
    match ty {
        FileType::Wal => {
            let mut at = HEADER_LEN;
            while at + FRAME_HEADER_LEN <= bytes.len() {
                let end = at + FRAME_HEADER_LEN + le_u32(bytes, at) as usize;
                if end > bytes.len() {
                    break;
                }
                // The frame header and the first payload bytes (op count and
                // first operation); the rest of the payload is user data.
                let targets = at..(at + FRAME_HEADER_LEN + 3).min(end);
                out.push(Unit { start: at, end, targets, skip: Some(at + 4..at + 8), seal: Seal::Frame });
                at = end;
            }
        }
        FileType::Manifest => {
            let generation = le_u64(bytes, 56);
            let mut at = HEADER_LEN;
            while at + RECORD_HEADER_LEN <= bytes.len() {
                let end = at + RECORD_HEADER_LEN + le_u32(bytes, at) as usize + SLOT_LEN;
                if end > bytes.len() {
                    break;
                }
                out.push(Unit {
                    start: at,
                    end,
                    targets: at..end,
                    skip: Some(at + 4..at + 8),
                    seal: Seal::Record { generation },
                });
                at = end;
            }
        }
        FileType::Sst => {
            let number = file_number(name);
            let footer_at = bytes.len().saturating_sub(FOOTER_LEN);
            let Ok(footer) = decode_footer(&bytes[footer_at..], number, footer_at as u64) else {
                return out;
            };
            out.push(Unit {
                start: footer_at,
                end: bytes.len(),
                targets: footer_at..footer_at + FOOTER_LEN - 4,
                skip: None,
                seal: Seal::Footer,
            });
            let slice = |h: kerno_storage::format::sst::BlockHandle| {
                let (s, e) = (h.offset as usize, h.offset.checked_add(h.len)? as usize);
                (h.len as usize > BLOCK_TRAILER_LEN && e <= bytes.len()).then_some(s..e)
            };
            // The filter block is a rewrite target again (finding F-E9A-03-1): a
            // rewrite of its bits that hides keys from `get` is tolerated by
            // `read_back` only when `verify()` reports it.
            for h in [footer.index, footer.props, footer.filter, footer.rdel] {
                if let Some(r) = slice(h) {
                    out.push(Unit {
                        start: r.start,
                        end: r.end,
                        targets: r.start..r.end - 4,
                        skip: None,
                        seal: Seal::Block,
                    });
                }
            }
            let index = slice(footer.index)
                .and_then(|r| {
                    open_block(&bytes[r.clone()], number, r.start as u64, u64::MAX, "SST-O4").ok().map(|p| (r, p))
                })
                .and_then(|(r, p)| decode_index(p, r.start as u64).ok());
            for e in index.into_iter().flatten() {
                let Some(r) = slice(e.handle) else { continue };
                // Offset array and entry count at the end of the payload.
                let payload_end = r.end - BLOCK_TRAILER_LEN;
                let count = le_u32(bytes, payload_end - 4) as usize;
                let tail = count.saturating_add(1).saturating_mul(4).min(payload_end - r.start);
                out.push(Unit {
                    start: r.start,
                    end: r.end,
                    targets: payload_end - tail..payload_end,
                    skip: None,
                    seal: Seal::Block,
                });
            }
        }
    }
    out
}

/// A position of `u.targets` that is not in `u.skip`.
fn pick(rng: &mut Rng, u: &Unit) -> usize {
    let skipped = u.skip.as_ref().map_or(0, Range::len);
    let k = rng.below((u.targets.len() - skipped).max(1) as u64) as usize;
    let at = u.targets.start + k;
    match &u.skip {
        Some(s) if at >= s.start => at + s.len(),
        _ => at,
    }
}

fn reseal(bytes: &mut [u8], u: &Unit, number: u64) {
    let unit = &mut bytes[u.start..u.end];
    match u.seal {
        // A rewritten header_len that no longer fits cannot be resealed: the
        // header is then left with its stale CRC, which is still a hostile file.
        Seal::Header => drop(reseal_header(unit)),
        Seal::Block => reseal_block(unit, number, u.start as u64),
        Seal::Footer => reseal_footer(unit, number),
        Seal::Frame => reseal_frame(unit, number),
        Seal::Record { generation } => reseal_record(unit, generation),
    }
}

/// Whether the unit starting at `start` is the filter block of the table
/// `old` (the unmutated bytes of an SST).
fn is_filter_block(old: &[u8], number: u64, start: usize) -> bool {
    let footer_at = old.len().saturating_sub(FOOTER_LEN);
    decode_footer(&old[footer_at..], number, footer_at as u64).is_ok_and(|f| f.filter.offset as usize == start)
}

/// The directory of the base image with one mutation of one file of type `ty`
/// chosen by `seed`, a description of it, and whether it rewrote the bits of a
/// filter block (with the CRC recomputed).
fn mutate(ty: FileType, seed: u64) -> (DirImage, String, bool) {
    let mut image = base().image.clone();
    let mut rng = Rng::new(seed);
    let files = files_with(&image, ty.ext());
    let victim = files[rng.below(files.len() as u64) as usize].clone();
    let old = image.files[&victim].clone();
    let mut bytes = old.clone();
    let mut filter_rewritten = false;
    let what = match rng.below(3) {
        0 if !bytes.is_empty() => {
            let at = rng.below(bytes.len() as u64) as usize;
            let bit = rng.below(8);
            bytes[at] ^= 1 << bit;
            format!("flip bit {bit} at {at}")
        }
        1 => {
            let len = rng.below(bytes.len() as u64 + 1) as usize;
            bytes.truncate(len);
            format!("truncate to {len}")
        }
        _ => {
            let all = units(ty, &victim, &old);
            let u = all[rng.below(all.len() as u64) as usize].clone();
            let n = rng.range(1, 5);
            let mut at = Vec::new();
            for _ in 0..n {
                let p = pick(&mut rng, &u);
                bytes[p] = [0x00, 0xff, rng.next_u64() as u8][rng.below(3) as usize];
                at.push(p);
            }
            reseal(&mut bytes, &u, file_number(&victim));
            filter_rewritten = matches!(ty, FileType::Sst) && is_filter_block(&old, file_number(&victim), u.start);
            format!("rewrite bytes {at:?} of the unit {}..{} ({:?}) and reseal", u.start, u.end, u.seal)
        }
    };
    image.files.insert(victim.clone(), bytes);
    (image, format!("{} of {}", what, victim.display()), filter_rewritten)
}

// ---- the probe ---------------------------------------------------------------

#[track_caller]
fn accepted(case: &str, step: &str, e: &Error) {
    assert!(matches!(e, Error::Corruption(_) | Error::Incompatible(_)), "{case}: {step} failed with {e:?}");
}

/// Whether the engine's answer, read through `snap`, is the model at the
/// sequence it reports: `Ok(n)` with `n` the number of keys whose `get`
/// disagrees with the model while the scan agrees (always 0 unless
/// `tolerate_get` is set, which only a rewritten filter block allows: `get`
/// consults the filter and the scan does not). `Err` is a reported error
/// (already checked).
fn read_back(case: &str, snap: &impl Snapshot, models: &[Model], tolerate_get: bool) -> Result<u32, ()> {
    let seq = snap.seq() as usize;
    assert!(seq < models.len(), "{case}: sequence {seq} was never assigned (last {})", models.len() - 1);
    let seen = match read_all(snap) {
        Ok(m) => m,
        Err(e) => {
            accepted(case, "scan", &e);
            return Err(());
        }
    };
    assert!(seen == models[seq], "{case}: the scan at sequence {seq} is not the model: {}", diff(&models[seq], &seen));
    let mut disagreeing = 0;
    for (k, v) in &models[seq] {
        match snap.get(k) {
            Ok(got) if got.as_ref() == Some(v) => {}
            Ok(got) => {
                assert!(tolerate_get, "{case}: get({k:?}) returned wrong data: {got:?}");
                disagreeing += 1;
            }
            Err(e) => {
                accepted(case, "get", &e);
                return Err(());
            }
        }
    }
    match snap.get(b"no-such-key") {
        Ok(got) => assert_eq!(got, None, "{case}: a key that was never written"),
        Err(e) => accepted(case, "get", &e),
    }
    Ok(disagreeing)
}

/// Returns whether the open succeeded and read back the model, and how many
/// `get`s disagreed with it (only when `tolerate_get`; then `verify()` must
/// have reported the directory).
fn probe(case: &str, image: &DirImage, access: AccessMode, tolerate_get: bool) -> (bool, u32) {
    let models = &base().models;
    // verify() reads and never writes.
    let disk = SimVfs::restore(image);
    let before = disk.dump();
    let flagged = match verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions::default()) {
        Err(e) => {
            accepted(case, "verify", &e);
            true
        }
        Ok(report) => !report.is_ok(),
    };
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);

    let disk = SimVfs::restore(image);
    let before = disk.dump();
    let (ok, disagreeing) = match open_in(&disk.as_vfs(), access) {
        Err(e) => {
            accepted(case, "open", &e);
            (false, 0)
        }
        Ok(db) => match db.snapshot() {
            Err(e) => {
                accepted(case, "snapshot", &e);
                (false, 0)
            }
            Ok(snap) => match read_back(case, &snap, models, tolerate_get) {
                Ok(n) => (n == 0, n),
                Err(()) => (false, 0),
            },
        },
    };
    assert!(disagreeing == 0 || flagged, "{case}: get() disagrees with the scan and verify() reported nothing");
    if access == AccessMode::ReadOnly {
        assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    }
    (ok, disagreeing)
}

/// One case: mutate, then read-write or read-only open plus `verify()`.
fn case(ty: FileType, access: AccessMode, seed: u64) -> bool {
    let (image, what, filter_rewritten) = mutate(ty, seed);
    let label = format!("{ty:?}/{access:?} seed {seed}: {what}");
    probe(&label, &image, access, filter_rewritten).0
}

fn proptest_config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
    ProptestConfig { cases, failure_persistence: None, max_shrink_iters: 0, ..ProptestConfig::default() }
}

/// The seed of the case, or the one in `OPEN_PROPERTIES_SEED`.
fn seed_strategy() -> BoxedStrategy<u64> {
    match std::env::var("OPEN_PROPERTIES_SEED").ok().and_then(|s| s.parse::<u64>().ok()) {
        Some(s) => Just(s).boxed(),
        None => any::<u64>().boxed(),
    }
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn wal_read_write(seed in seed_strategy()) {
        case(FileType::Wal, AccessMode::ReadWrite, seed);
    }

    #[test]
    fn wal_read_only(seed in seed_strategy()) {
        case(FileType::Wal, AccessMode::ReadOnly, seed);
    }

    #[test]
    fn sst_read_write(seed in seed_strategy()) {
        case(FileType::Sst, AccessMode::ReadWrite, seed);
    }

    #[test]
    fn sst_read_only(seed in seed_strategy()) {
        case(FileType::Sst, AccessMode::ReadOnly, seed);
    }

    #[test]
    fn manifest_read_write(seed in seed_strategy()) {
        case(FileType::Manifest, AccessMode::ReadWrite, seed);
    }

    #[test]
    fn manifest_read_only(seed in seed_strategy()) {
        case(FileType::Manifest, AccessMode::ReadOnly, seed);
    }
}

// ---- finding F-E9A-03-1 -----------------------------------------------------

/// Replaces the bits of the filter block of every table of `image` with
/// `bits(table, entries)` (same length, the 8-byte filter header kept) and
/// reseals the block, as someone who can recompute a CRC would. Returns the
/// offset of the filter block of each rewritten table, by file name.
fn rewrite_filters(image: &mut DirImage, bits: impl Fn(&SstMeta, &[Entry]) -> Vec<u8>) -> BTreeMap<String, u64> {
    let mut at = BTreeMap::new();
    for name in files_with(image, "sst") {
        let number = file_number(&name);
        let bytes = image.files.get_mut(&name).unwrap();
        let ex = SstExpect { file_number: number, db_id: DB_ID, meta: None, last_seq: u64::MAX };
        let (sst, entries) = read_sst(bytes, &ex).unwrap();
        if sst.index.is_empty() {
            continue; // only range tombstones: no point key for a filter to hide
        }
        let new = bits(&sst, &entries);
        let (s, e) = (sst.footer.filter.offset as usize, (sst.footer.filter.offset + sst.footer.filter.len) as usize);
        assert_eq!(new.len(), e - BLOCK_TRAILER_LEN - s - FILTER_HEADER_LEN, "the rewrite keeps the block's size");
        bytes[s + FILTER_HEADER_LEN..e - BLOCK_TRAILER_LEN].copy_from_slice(&new);
        reseal_block(&mut bytes[s..e], number, s as u64);
        at.insert(name.file_name().unwrap().to_string_lossy().into_owned(), s as u64);
    }
    assert!(at.len() >= 3, "the base needs several tables with point keys");
    at
}

/// No key or value byte of the base in the text of an `Issue` or an `Error`
/// (I-29; tests/redaction.rs checks the same over its own database).
#[track_caller]
fn assert_no_content(text: &str) {
    assert!(!text.contains("key-0"), "a key in {text}");
}

/// Finding F-E9A-03-1, the coarse case. Every filter is cleared and its block
/// resealed with a valid CRC: before the fix, open succeeded and `get` hid 307
/// of 357 keys with no error (a scan, which does not use the filter, returned
/// them). Now the open refuses the table (SST-V3 over the last key of each
/// data block, which the open already holds in the index), and `verify()`
/// reports it in both modes, at the filter block, with no key bytes.
#[test]
fn resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify() {
    let mut image = base().image.clone();
    let blocks = rewrite_filters(&mut image, |sst, _| vec![0; sst.filter.bits.len()]);
    for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
        let disk = SimVfs::restore(&image);
        match open_in(&disk.as_vfs(), access) {
            Err(Error::Corruption(msg)) => {
                assert!(msg.contains("SST-V3"), "{access:?}: {msg}");
                assert_no_content(&msg);
            }
            Err(e) => panic!("{access:?}: open failed with {e:?}, expected Corruption"),
            Ok(_) => panic!("{access:?}: a table whose filter lost every bit opened"),
        }
    }
    for deep in [true, false] {
        let disk = SimVfs::restore(&image);
        let report = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions { deep, check_wal: true }).unwrap();
        let reported: BTreeMap<String, (&str, Option<u64>)> =
            report.issues.iter().map(|i| (i.file.clone(), (i.rule, i.offset))).collect();
        let expected: BTreeMap<String, (&str, Option<u64>)> =
            blocks.iter().map(|(f, &at)| (f.clone(), ("SST-V3", Some(at)))).collect();
        assert_eq!(reported, expected, "deep={deep}: one SST-V3 per table, at its filter block");
        assert_no_content(&format!("{:?}", report.issues));
    }
}

/// THE REMAINING LIMIT of finding F-E9A-03-1, kept on purpose: a filter that
/// lacks only keys in the middle of data blocks (every block's last key is
/// still in it) opens, in both modes, and `get` then answers "not found" for
/// keys the table holds (or an older version of them) with no error, while
/// the scan returns the model. Nothing that `get` can afford detects it: it
/// would have to read the data block to distrust each "no", which is what the
/// filter exists to avoid, and a deliberate rewrite is only distinguishable
/// from the real filter with authentication (ADR-0006, pending). What the
/// engine does offer is `verify()` with `deep`, which reads every key and
/// reports SST-V3 for each table, with no key bytes; without `deep` it reports
/// nothing. Do not "fix" this test by making the open stricter: assert the
/// new limit here instead.
#[test]
fn known_limit_get_trusts_a_resealed_filter_missing_interior_keys() {
    let b = base();
    let mut image = b.image.clone();
    let blocks = rewrite_filters(&mut image, |sst, entries| {
        let last: Vec<&[u8]> = sst.index.iter().map(|e| e.last_key.as_slice()).collect();
        let mut bits = vec![0u8; sst.filter.bits.len()];
        let mut dropped = false;
        for e in entries {
            let h = bloom::hash(&e.key);
            // Keep every block's last key, drop every other of the rest.
            if last.contains(&e.key.as_slice()) || dropped {
                bloom::insert(&mut bits, sst.filter.k, h);
            }
            if !last.contains(&e.key.as_slice()) {
                dropped = !dropped;
            }
        }
        bits
    });
    let model = b.models.last().unwrap();
    for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
        let disk = SimVfs::restore(&image);
        let db = open_in(&disk.as_vfs(), access).unwrap_or_else(|e| panic!("{access:?}: open failed: {e}"));
        let snap = db.snapshot().unwrap();
        assert!(&read_all(&snap).unwrap() == model, "{access:?}: the scan does not use the filter");
        let wrong = model.iter().filter(|(k, v)| snap.get(k).unwrap().as_ref() != Some(*v)).count();
        assert!(wrong > 0, "{access:?}: the limit is gone: get() found every key (see the doc of this test)");
    }
    let disk = SimVfs::restore(&image);
    let deep = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions::default()).unwrap();
    let reported: BTreeMap<String, &str> = deep.issues.iter().map(|i| (i.file.clone(), i.rule)).collect();
    let expected: BTreeMap<String, &str> = blocks.keys().map(|f| (f.clone(), "SST-V3")).collect();
    assert_eq!(reported, expected, "deep verify reports each table whose filter lacks a key");
    assert_no_content(&format!("{:?}", deep.issues));
    // Without `deep` nothing reads a key: the report is clean.
    let shallow = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions { deep: false, check_wal: true }).unwrap();
    assert!(shallow.is_ok(), "{:?}", shallow.issues);
}

// ---- the properties are not vacuous -------------------------------------------

#[test]
fn control_unmutated_directory_opens_to_the_final_model_in_both_modes() {
    let b = base();
    let last = b.models.last().unwrap();
    for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
        let disk = SimVfs::restore(&b.image);
        let db = open_in(&disk.as_vfs(), access).unwrap();
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq() as usize, b.models.len() - 1, "{access:?}");
        assert!(&read_all(&snap).unwrap() == last, "{access:?}");
    }
    let disk = SimVfs::restore(&b.image);
    let report = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions::default()).unwrap();
    assert!(report.is_ok(), "{:?}", report.issues);
}

/// Over a fixed batch of seeds per file type, the mutations reach both
/// outcomes (damage reported, and a mutated directory that still opens to a
/// model state) and change the file: a generator that produced only no-ops or
/// only unreadable files would make the properties above pass for nothing.
#[test]
fn mutations_reach_both_outcomes_and_change_the_file() {
    let mut by_type: BTreeMap<&str, (u32, u32, u32)> = BTreeMap::new();
    for ty in [FileType::Wal, FileType::Sst, FileType::Manifest] {
        let (mut reported, mut opened, mut changed) = (0, 0, 0);
        for seed in 0..300u64 {
            let (image, _, _) = mutate(ty, seed);
            changed += u32::from(image.files != base().image.files);
            if case(ty, AccessMode::ReadWrite, seed) {
                opened += 1;
            } else {
                reported += 1;
            }
        }
        by_type.insert(ty.ext(), (reported, opened, changed));
    }
    for (ext, (reported, opened, changed)) in by_type {
        assert!(reported > 0, "{ext}: no mutation was ever reported");
        assert!(opened > 0, "{ext}: no mutated directory ever opened");
        assert!(changed > 200, "{ext}: only {changed} of 300 mutations changed the file");
    }
}
