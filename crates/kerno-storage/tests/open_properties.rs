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
//!   never a value that was not written;
//! * `verify()` and the read-only open leave the directory unchanged.
//!
//! Threshold (acta §5): 2 000 cases per file type and per mode.
//!
//! Seeding. The default run is deterministic: proptest draws the case seeds
//! from a fixed base seed (`DEFAULT_BASE_SEED`), so every run of every CI job
//! on the same `Cargo.lock` executes the same 2 000 cases per property and a
//! pull request cannot go red on a case nobody has seen. (Before this, the
//! proptest RNG seed was `RngSeed::Random`, taken from the operating system on
//! every run, so each run drew fresh cases.) The cases themselves are
//! `u64` seeds; a failure prints `seed = <n>` and the label of the case, which
//! ends with the command that re-runs it. `failure_persistence` is off, so no
//! regression file is written or read. A proptest upgrade in `Cargo.lock`
//! changes the sequence drawn from the base seed: explore widely when bumping.
//!
//! Exploring further (these do not run in CI):
//!
//! * more cases: `PROPTEST_CASES=50000 cargo test -p kerno-storage --release
//!   --test open_properties`;
//! * other cases: add `OPEN_PROPERTIES_BASE_SEED=<u64>` (for instance
//!   `$RANDOM$RANDOM`); the same base seed and case count repeat the same
//!   cases;
//! * one case: `OPEN_PROPERTIES_SEED=<seed>` (all six properties then run only
//!   that seed).
//!
//! A failure found this way is either a defect of `src/` (pin it as a named
//! known-failure test and exclude its class here, as for F-E9A-03-1) or a
//! defect of the model or mutation (fix it here, as for the WAL payload below).
//!
//! Limit of the rewrite: a CRC cannot tell a rewritten user value from the
//! real one, so rewrites with a recomputed CRC target only structure: the
//! common headers, the footer, the meta blocks, the offset array and count of
//! a data block, the 32-byte header of a WAL frame, and the MANIFEST records.
//! A rewrite of a value byte would make the "no different data" property
//! unsatisfiable by design. The same holds for the payload of a WAL frame,
//! including its first bytes: they are the operation kind and the lengths of
//! the key and the value, so a resealed rewrite re-reads the same bytes as
//! another well-formed commit that the engine rightly applies. See
//! `resealed_wal_payload_rewrite_is_another_valid_commit_not_damage`.
//!
//! Mutation record (acta §3.2), run on a copy of the tree: `open_block`
//! (src/format/block.rs) with its CRC comparison replaced by `false && ...`.
//! Result: `sst_read_write`, `sst_read_only` and
//! `mutations_reach_both_outcomes_and_change_the_file` fail ("the scan at
//! sequence 409 is not the model: wrong value for key-00096", seed
//! 1618401364763173671); the other six tests stay green. Nothing was mutated in
//! the working tree.
//!
//! Finding F-E9A-03-1 (pinned, not fixed): see
//! `known_failure_resealed_filter_hides_existing_keys`.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::format::block::{BLOCK_TRAILER_LEN, open_block, reseal_block};
use kerno_storage::format::header::{HEADER_LEN, reseal_header};
use kerno_storage::format::manifest::{RECORD_HEADER_LEN, reseal_record};
use kerno_storage::format::sst::{FOOTER_LEN, decode_footer, decode_index, reseal_footer};
use kerno_storage::format::wal::{FRAME_HEADER_LEN, check_frame, reseal_frame};
use kerno_storage::{AccessMode, Background, CloseBudget, Config, KernoDb, TestConfig, VerifyOptions, verify};
use kerno_testkit::harness::{Model, apply, diff, read_all};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{DirImage, SimVfs, assert_dir_unchanged};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{RngSeed, TestRunner};

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
                // The frame header only. The payload is user data: even its
                // first bytes (operation kind, key length) can be rewritten
                // into another well-formed commit (see the head of this file).
                let targets = at..at + FRAME_HEADER_LEN;
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
            // The filter block is left out of the rewrites: see
            // `known_failure_resealed_filter_hides_existing_keys` (finding F-E9A-03-1).
            for h in [footer.index, footer.props, footer.rdel] {
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

/// The directory of the base image with one mutation of one file of type `ty`
/// chosen by `seed`, and a description of it.
fn mutate(ty: FileType, seed: u64) -> (DirImage, String) {
    let mut image = base().image.clone();
    let mut rng = Rng::new(seed);
    let files = files_with(&image, ty.ext());
    let victim = files[rng.below(files.len() as u64) as usize].clone();
    let old = image.files[&victim].clone();
    let mut bytes = old.clone();
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
            format!("rewrite bytes {at:?} of the unit {}..{} ({:?}) and reseal", u.start, u.end, u.seal)
        }
    };
    image.files.insert(victim.clone(), bytes);
    (image, format!("{} of {}", what, victim.display()))
}

// ---- the probe ---------------------------------------------------------------

#[track_caller]
fn accepted(case: &str, step: &str, e: &Error) {
    assert!(matches!(e, Error::Corruption(_) | Error::Incompatible(_)), "{case}: {step} failed with {e:?}");
}

/// Whether the engine's answer, read through `snap`, is the model at the
/// sequence it reports. `Err` is a reported error (already checked).
fn read_back(case: &str, snap: &impl Snapshot, models: &[Model]) -> Result<(), ()> {
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
    for (k, v) in &models[seq] {
        match snap.get(k) {
            Ok(got) => assert_eq!(got.as_ref(), Some(v), "{case}: get({k:?}) returned wrong data"),
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
    Ok(())
}

/// Returns whether the open succeeded and read back the model.
fn probe(case: &str, image: &DirImage, access: AccessMode) -> bool {
    let models = &base().models;
    // verify() reads and never writes.
    let disk = SimVfs::restore(image);
    let before = disk.dump();
    if let Err(e) = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions::default()) {
        accepted(case, "verify", &e);
    }
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);

    let disk = SimVfs::restore(image);
    let before = disk.dump();
    let ok = match open_in(&disk.as_vfs(), access) {
        Err(e) => {
            accepted(case, "open", &e);
            false
        }
        Ok(db) => match db.snapshot() {
            Err(e) => {
                accepted(case, "snapshot", &e);
                false
            }
            Ok(snap) => read_back(case, &snap, models).is_ok(),
        },
    };
    if access == AccessMode::ReadOnly {
        assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    }
    ok
}

/// One case: mutate, then read-write or read-only open plus `verify()`.
fn case(ty: FileType, access: AccessMode, seed: u64) -> bool {
    let (image, what) = mutate(ty, seed);
    let label = format!(
        "{ty:?}/{access:?} seed {seed}: {what} [re-run: OPEN_PROPERTIES_SEED={seed} cargo test -p kerno-storage \
         --release --test open_properties]"
    );
    probe(&label, &image, access)
}

/// Base seed of the default run: the case seeds of CI come from it.
const DEFAULT_BASE_SEED: u64 = 0x4b45_524e_4f5f_4539;

/// An unsigned integer from the environment. A value that does not parse is a
/// panic, not a silent fallback: a mistyped seed must not turn into a
/// different run.
fn env_u64(name: &str) -> Option<u64> {
    let v = std::env::var(name).ok()?;
    Some(v.trim().parse().unwrap_or_else(|_| panic!("{name}={v:?} is not an unsigned integer")))
}

fn proptest_config() -> ProptestConfig {
    let cases = env_u64("PROPTEST_CASES").map_or(2000, |n| u32::try_from(n).expect("PROPTEST_CASES fits in u32"));
    ProptestConfig {
        cases,
        // Fixed, so that a run draws the same cases as any other run: see the
        // head of this file. Never `RngSeed::Random`.
        rng_seed: RngSeed::Fixed(env_u64("OPEN_PROPERTIES_BASE_SEED").unwrap_or(DEFAULT_BASE_SEED)),
        failure_persistence: None,
        max_shrink_iters: 0,
        ..ProptestConfig::default()
    }
}

/// The seed of the case, or the one in `OPEN_PROPERTIES_SEED`.
fn seed_strategy() -> BoxedStrategy<u64> {
    match env_u64("OPEN_PROPERTIES_SEED") {
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

/// KNOWN FAILURE, pinned (finding F-E9A-03-1, not fixed here: src/ is out of
/// scope of E9A-03). A bloom filter block whose bits were rewritten and
/// resealed passes every check at open, and `get` then answers "not found" for
/// keys the table holds, with no error, while a scan of the same table
/// returns them. The rewrite properties above leave the filter block out of
/// their targets for this reason; bit flips and truncations still reach it
/// (the CRC catches those).
///
/// This test asserts the CURRENT WRONG behaviour. When the engine is fixed
/// (the open or the read must report Corruption, or `get` must not trust the
/// filter alone) it fails: invert it into a regression test and put the
/// filter back into the rewrite targets of `units`.
#[test]
fn known_failure_resealed_filter_hides_existing_keys() {
    let b = base();
    let mut image = b.image.clone();
    for name in files_with(&image, "sst") {
        let number = file_number(&name);
        let bytes = image.files.get_mut(&name).unwrap();
        let at = bytes.len() - FOOTER_LEN;
        let footer = decode_footer(&bytes[at..], number, at as u64).unwrap();
        let (s, e) = (footer.filter.offset as usize, (footer.filter.offset + footer.filter.len) as usize);
        // Keep the 8-byte filter header; clear every bit.
        bytes[s + 8..e - BLOCK_TRAILER_LEN].fill(0);
        reseal_block(&mut bytes[s..e], number, s as u64);
    }
    let model = b.models.last().unwrap();
    for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
        let disk = SimVfs::restore(&image);
        let db = open_in(&disk.as_vfs(), access).unwrap_or_else(|e| panic!("{access:?}: open failed: {e}"));
        let snap = db.snapshot().unwrap();
        assert!(&read_all(&snap).unwrap() == model, "{access:?}: the scan does not use the filter");
        let hidden = model.keys().filter(|k| snap.get(k).unwrap().is_none()).count();
        assert!(hidden > 0, "{access:?}: the defect is gone: get() found every key (see the doc of this test)");
        println!("{access:?}: get() hid {hidden} of {} keys that a scan returns", model.len());
    }
    let disk = SimVfs::restore(&image);
    let report = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions::default()).unwrap();
    println!("verify on the same directory: {} issue(s) {:?}", report.issues.len(), report.issues.first());
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
            let (image, _) = mutate(ty, seed);
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

// ---- the WAL payload is not a rewrite target ----------------------------------

/// The file and byte range of the frame of `image` whose first commit starts
/// with a point operation on `key`.
fn wal_frame_of(image: &DirImage, key: &str) -> (PathBuf, Range<usize>) {
    for name in files_with(image, "wal") {
        let bytes = &image.files[&name];
        for u in units(FileType::Wal, &name, bytes).into_iter().skip(1) {
            let frame = &bytes[u.start..u.end];
            let Ok(f) = check_frame(frame, u.start as u64, false, le_u64(frame, 8)) else { continue };
            if f.commits[0].points.first().is_some_and(|(k, _)| k == key.as_bytes()) {
                return (name, u.start..u.end);
            }
        }
    }
    panic!("no WAL frame starts with a point operation on {key}");
}

/// Root cause of the WAL failures that `wal_read_write` and `wal_read_only`
/// used to hit at random (seeds 17787922088759419750, 23721, 30333, 213101,
/// 219857, 360354, 437914 and 444753; 7 of the seeds 0..500 000, about 1 case
/// in 70 000 per property, so roughly 1 CI run in 20 when the cases were
/// random), shown as a fact: this is NOT a defect of `src/` but of the old
/// model of the test. Every one of those seeds rewrites frame byte 33 or 34.
///
/// The old rewrite also targeted the first payload bytes of a frame. In the
/// base, every frame is one commit of one operation whose value is 200 equal
/// bytes. Rewriting, with the CRC resealed:
///
/// * the operation kind `PUT` (0x01) to `DELETE_RANGE` (0x03), at frame byte 33
///   (seeds 17787922088759419750, 30333): the key and the 200-byte value are
///   now read as the two bounds of a range; `key-… < "xxx…"`, so the commit is
///   canonical and deletes nothing that existed, and the put is gone;
/// * the key length 9 to 0x66, at frame byte 34 (seed 23721): the longer key
///   swallows the length bytes and part of the equal value bytes, and what is
///   left reads as a 108-byte value that ends exactly at the end of the
///   payload.
///
/// In both, `check_frame` accepts the frame (every WAL-C4 rule holds and the
/// payload is consumed to its last byte), so no format rule is broken: the
/// bytes are a different valid history, and the engine applies exactly it, in
/// both modes, with `verify()` reporting nothing. A CRC cannot tell this from
/// the original, as for a rewritten value. The test pins that, per case: the
/// rewritten frame is valid and different, the engine opens at the same
/// sequence and shows the model with exactly that commit's effect.
#[test]
fn resealed_wal_payload_rewrite_is_another_valid_commit_not_damage() {
    let b = base();
    let last_seq = b.models.len() - 1;
    let last = &b.models[last_seq];
    // (key of the frame, byte of the frame, new value of that byte)
    for (key, at, byte) in [("key-00376", 33, 0x03u8), ("key-00371", 33, 0x03), ("key-00364", 34, 0x66)] {
        let mut image = b.image.clone();
        let (name, r) = wal_frame_of(&image, key);
        let number = file_number(&name);
        let frame = &mut image.files.get_mut(&name).unwrap()[r.clone()];
        let first_seq = le_u64(frame, 8);
        let before = check_frame(frame, r.start as u64, false, first_seq).unwrap();
        assert_ne!(frame[at], byte, "{key}: the rewrite changes nothing");
        frame[at] = byte;
        reseal_frame(frame, number);
        let after = check_frame(frame, r.start as u64, false, first_seq)
            .unwrap_or_else(|e| panic!("{key}: the rewritten frame is damaged after all: {e:?}"));
        assert_ne!(after.commits, before.commits, "{key}: the rewrite did not change the commit");

        // The model with the effect of the rewritten commit instead of the original.
        let older = &b.models[first_seq as usize - 1];
        let mut want = last.clone();
        want.remove(key.as_bytes());
        for c in &after.commits {
            for (s, e) in &c.ranges {
                assert!(older.keys().all(|k| !(k >= s && k < e)), "{key}: the range deletes data that existed");
            }
            for (k, v) in &c.points {
                match v {
                    Some(v) => want.insert(k.clone(), v.clone()),
                    None => want.remove(k),
                };
            }
        }
        assert!(&want != last, "{key}: the expected model did not change");

        for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
            let disk = SimVfs::restore(&image);
            let db = open_in(&disk.as_vfs(), access).unwrap_or_else(|e| panic!("{key} {access:?}: open failed: {e}"));
            let snap = db.snapshot().unwrap();
            assert_eq!(snap.seq() as usize, last_seq, "{key} {access:?}");
            let seen = read_all(&snap).unwrap();
            assert!(
                seen == want,
                "{key} {access:?}: the engine does not show the rewritten commit: {}",
                diff(&want, &seen)
            );
            assert_eq!(snap.get(key.as_bytes()).unwrap(), None, "{key} {access:?}");
        }
        let disk = SimVfs::restore(&image);
        let report = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions::default()).unwrap();
        assert!(report.is_ok(), "{key}: verify() reports a valid commit: {:?}", report.issues);
    }
}

/// The mutation side of the same fact: no WAL rewrite target leaves the
/// 32-byte frame header (and at least one frame exists).
#[test]
fn wal_rewrite_targets_are_the_frame_header_only() {
    let image = &base().image;
    let mut frames = 0;
    for name in files_with(image, "wal") {
        for u in units(FileType::Wal, &name, &image.files[&name]).iter().skip(1) {
            assert_eq!(u.targets, u.start..u.start + FRAME_HEADER_LEN, "{}", name.display());
            frames += 1;
        }
    }
    assert!(frames > 0, "the base has no WAL frame");
}

/// The default run must not depend on the operating system's entropy: two
/// runners built from the config draw the same seeds, and the config pins the
/// RNG seed. With `RngSeed::Random` (the old behaviour) both fail.
#[test]
fn default_run_draws_the_same_case_seeds_every_time() {
    assert!(matches!(proptest_config().rng_seed, RngSeed::Fixed(_)), "the proptest RNG seed is not fixed");
    let draw = || {
        let strategy = seed_strategy();
        let mut runner = TestRunner::new(proptest_config());
        (0..16).map(|_| strategy.new_tree(&mut runner).unwrap().current()).collect::<Vec<u64>>()
    };
    assert_eq!(draw(), draw());
}
