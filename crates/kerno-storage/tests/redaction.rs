//! I-29 for `kerno-storage` (acta E9A-02): errors, `verify()` reports,
//! `inspect()` and metrics carry no key or value bytes (ADR-0015, T-08).
//!
//! The test writes keys and values that carry sentinels, provokes every one
//! of the seven variants of `kerno_core::Error`, runs `verify()` and opens
//! over damaged copies of the files, reads `inspect()` and the metrics, and
//! scans the `Display` and `Debug` of everything it saw for the sentinels in
//! every form a formatter could print them (bytes read as text, hex, decimal
//! lists, `\x` escapes). It fails if a variant was not produced, so a
//! scenario that silently stops reaching an error path cannot pass.
//! Each engine configuration the storage tests cover runs it: the default
//! (background threads), the `kerno-inline`, `kerno-m0` and `kerno-serial`
//! profiles of `kerno_storage::testing`, and buffered durability.
//!
//! Mutation check (acta §3.2), recorded when the test was written. The
//! mutation: `Table::open` (`src/sst.rs`, the "referenced by the MANIFEST but
//! missing" error) got `(key {:?})` with `meta.smallest` appended to its
//! message. It was applied to `src/sst.rs` in place, reverted by hand and the
//! revert confirmed with `git diff` (empty); no mutation stays in the tree.
//!
//! * First run, scenario without "vanishing tables": all six tests stayed
//!   GREEN. Cause: a table that is missing from the directory is caught by
//!   the listing check (`recovery::classify`, REC-5) before `Table::open`
//!   runs, so the mutated line was never reached. The test was incomplete,
//!   not the invariant satisfied.
//! * Fix: `vanishing_tables` removes a listed table at the moment the open
//!   reads it, which is the one way `Table::open` meets a missing file.
//! * Second run, same mutation: the five configuration tests FAILED
//!   ("N of M texts carry a sentinel (I-29)", the leaked text being
//!   `...000010.sst: referenced by the MANIFEST but missing (key [107, 101,
//!   ...` in `Display`, `Debug` and pretty `Debug`), and the detector test
//!   stayed green, as it should. Without the mutation: six of six pass.

use std::collections::BTreeSet;
use std::fmt::{Debug, Display};
use std::io;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, Result, WriteBatch};
use kerno_storage::format::header::{HEADER_LEN, reseal_header};
use kerno_storage::testing::{Inline, M0, Profile, Serial};
use kerno_storage::{AccessMode, Config, KernoDb, VerifyOptions, inspect, verify};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{DirImage, HookAction, HookOpKind, HookPoint, HookVfs, SimVfs};

const DIR: &str = "/db";
const DB_ID: [u8; 16] = [0x5e; 16];

/// A recognisable byte string. Two are plain ASCII, two are not valid UTF-8.
struct Sentinel {
    name: &'static str,
    bytes: &'static [u8],
}

const KEY_TEXT: Sentinel = Sentinel { name: "text key", bytes: b"kerno-redaction-key-7f3a9c" };
const KEY_BIN: Sentinel =
    Sentinel { name: "binary key", bytes: &[0xde, 0xad, 0xbe, 0xef, 0x5e, 0x17, 0xc0, 0xde, 0x42, 0x99] };
const VAL_TEXT: Sentinel = Sentinel { name: "text value", bytes: b"kerno-redaction-value-c4e81b" };
const VAL_BIN: Sentinel =
    Sentinel { name: "binary value", bytes: &[0xfe, 0xed, 0xfa, 0xce, 0x0b, 0xad, 0xf0, 0x0d, 0x77, 0x31] };
const SENTINELS: [&Sentinel; 4] = [&KEY_TEXT, &KEY_BIN, &VAL_TEXT, &VAL_BIN];

const VARIANTS: [&str; 7] = ["Io", "Corruption", "Locked", "InvalidArgument", "Poisoned", "Incompatible", "ReadOnly"];

// ------------------------------------------------------------- detector

/// Lower-cased and without whitespace, so `{:#?}` of a byte list (one number
/// per line) and `{:X?}` compare like their compact forms.
fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// Every form in which `bytes` could appear in a message, normalised.
fn forms(bytes: &[u8]) -> Vec<(&'static str, String)> {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let escaped: String = bytes.iter().map(|b| format!("\\x{b:02x}")).collect();
    let ascii_escaped: String = bytes.iter().flat_map(|&b| std::ascii::escape_default(b)).map(char::from).collect();
    let strip = |s: String| s.trim_start_matches('[').trim_end_matches(']').to_string();
    let forms = [
        ("text", String::from_utf8_lossy(bytes).into_owned()),
        ("text debug", format!("{:?}", String::from_utf8_lossy(bytes)).trim_matches('"').to_string()),
        ("hex", hex),
        ("0x hex", bytes.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(",")),
        ("escaped", escaped),
        ("ascii-escaped", ascii_escaped),
        ("decimal list", strip(format!("{bytes:?}"))),
        ("hex list", strip(format!("{bytes:x?}"))),
    ];
    forms.into_iter().map(|(n, f)| (n, norm(&f))).collect()
}

/// Which sentinels and forms `text` contains.
fn find_sentinels(text: &str) -> Vec<String> {
    let text = norm(text);
    let mut hits = Vec::new();
    for s in SENTINELS {
        for (form, needle) in forms(s.bytes) {
            if text.contains(&needle) {
                hits.push(format!("{} as {form}", s.name));
            }
        }
    }
    hits
}

/// What a scenario saw: every rendered text with where it came from, and the
/// `Error` variants produced.
#[derive(Default)]
struct Seen {
    texts: Vec<(String, String)>,
    variants: BTreeSet<&'static str>,
    issues: usize,
}

fn variant_name(e: &Error) -> &'static str {
    match e {
        Error::Io(_) => "Io",
        Error::Corruption(_) => "Corruption",
        Error::Locked => "Locked",
        Error::InvalidArgument(_) => "InvalidArgument",
        Error::Poisoned(_) => "Poisoned",
        Error::Incompatible(_) => "Incompatible",
        Error::ReadOnly(_) => "ReadOnly",
        // `Error` is non-exhaustive; a new variant must be added to VARIANTS.
        _ => "unknown",
    }
}

impl Seen {
    fn text(&mut self, label: &str, text: String) {
        self.texts.push((label.to_string(), text));
    }

    fn debug(&mut self, label: &str, v: &dyn Debug) {
        self.text(label, format!("{v:?}"));
        self.text(label, format!("{v:#?}"));
    }

    fn error(&mut self, label: &str, e: &Error) {
        self.variants.insert(variant_name(e));
        self.text(label, e.to_string());
        self.debug(label, e);
    }

    /// Records the error, or the `Debug` of the value.
    fn outcome<T: Debug>(&mut self, label: &str, r: &Result<T>) {
        match r {
            Ok(v) => self.debug(label, v),
            Err(e) => self.error(label, e),
        }
    }

    fn display(&mut self, label: &str, v: &dyn Display) {
        self.text(label, v.to_string());
    }

    /// Texts that contain a sentinel, one line each.
    fn leaks(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (label, text) in &self.texts {
            for hit in find_sentinels(text) {
                let shown: String = text.chars().take(160).collect();
                out.push(format!("[{label}] {hit}: {shown}"));
            }
        }
        out
    }
}

// ------------------------------------------------------------- fixtures

/// The configuration of a case: `Options` of the harness, the profile's
/// adjustments, and a fixed database id.
fn config_with(durability: Durability, adjust: fn(&mut Config)) -> Config {
    let mut cfg = Config::from(&Options { durability, ..Options::small_for_tests() });
    adjust(&mut cfg);
    cfg.test.db_id = Some(DB_ID);
    cfg
}

fn key(sentinel: &Sentinel, i: u32) -> Vec<u8> {
    let mut k = sentinel.bytes.to_vec();
    k.extend(format!("-{i:05}").bytes());
    k
}

fn value(sentinel: &Sentinel, i: u32) -> Vec<u8> {
    let mut v = sentinel.bytes.to_vec();
    v.extend(format!("-{i:05}").bytes());
    v.extend([b'.'; 150]);
    v
}

/// Every key starts with a key sentinel and every value with a value
/// sentinel, alternating text and binary, so any key a message names carries
/// one.
fn sentinel_batch(i: u32) -> WriteBatch {
    let (k, v) = if i.is_multiple_of(2) { (&KEY_TEXT, &VAL_TEXT) } else { (&KEY_BIN, &VAL_BIN) };
    let mut b = WriteBatch::new();
    b.put(key(k, i), value(v, i));
    if i % 7 == 6 {
        b.delete(key(if i.is_multiple_of(2) { &KEY_BIN } else { &KEY_TEXT }, i - 1));
    }
    if i % 50 == 49 {
        b.delete_range(key(&KEY_TEXT, i - 40), key(&KEY_TEXT, i - 30));
    }
    b
}

fn commit_range(db: &KernoDb, from: u32, to: u32) {
    for i in from..to {
        db.commit(&sentinel_batch(i)).unwrap();
    }
}

fn open(vfs: &Arc<dyn Vfs>, cfg: &Config) -> Result<KernoDb> {
    KernoDb::open(vfs.clone(), Path::new(DIR), cfg)
}

/// Everything a handle shows: metrics, stats, read-only reason, orphans.
fn observe(seen: &mut Seen, label: &str, db: &KernoDb) {
    seen.debug(&format!("{label}: metrics"), &db.metrics());
    seen.debug(&format!("{label}: stats"), &db.stats());
    seen.debug(&format!("{label}: read-only reason"), &db.is_read_only());
    seen.debug(&format!("{label}: orphans"), &db.recovery_orphans());
}

/// Reads the whole database through a snapshot, as a caller would.
fn read_everything(seen: &mut Seen, label: &str, db: &KernoDb) {
    let snap = match db.snapshot() {
        Ok(s) => s,
        Err(e) => return seen.error(&format!("{label}: snapshot"), &e),
    };
    let scan = snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX);
    seen.outcome(&format!("{label}: scan"), &scan.map(|rows| rows.len()));
    for i in [0u32, 1, 40, 41, 200, 201] {
        let k = key(if i.is_multiple_of(2) { &KEY_TEXT } else { &KEY_BIN }, i);
        seen.outcome(&format!("{label}: get"), &snap.get(&k).map(|v| v.map(|v| v.len())));
    }
}

// ------------------------------------------------------------- the probes

/// `verify` (deep and shallow), `inspect`, a read-write open with a full
/// read and a close, and a read-only open, each on its own copy of `image`.
fn probe(seen: &mut Seen, label: &str, image: &DirImage, cfg: &Config) {
    let dir = Path::new(DIR);
    for opts in [VerifyOptions::default(), VerifyOptions { deep: false, check_wal: false }] {
        let disk = SimVfs::restore(image);
        let report = verify(disk.as_vfs(), dir, &opts);
        seen.outcome(&format!("{label}: verify {opts:?}"), &report);
        if let Ok(r) = &report {
            for issue in &r.issues {
                seen.issues += 1;
                seen.debug(&format!("{label}: issue"), issue);
                seen.display(&format!("{label}: issue detail"), &issue.detail);
            }
        }
    }
    let disk = SimVfs::restore(image);
    seen.outcome(&format!("{label}: inspect"), &inspect(disk.as_vfs(), dir));

    let disk = SimVfs::restore(image);
    let vfs = disk.as_vfs();
    match open(&vfs, cfg) {
        Err(e) => seen.error(&format!("{label}: open"), &e),
        Ok(db) => {
            observe(seen, &format!("{label}: open"), &db);
            read_everything(seen, &format!("{label}: open"), &db);
            seen.outcome(&format!("{label}: close"), &db.close());
        }
    }

    let disk = SimVfs::restore(image);
    let ro = Config { access: AccessMode::ReadOnly, ..cfg.clone() };
    match open(&disk.as_vfs(), &ro) {
        Err(e) => seen.error(&format!("{label}: read-only open"), &e),
        Ok(db) => {
            observe(seen, &format!("{label}: read-only open"), &db);
            read_everything(seen, &format!("{label}: read-only open"), &db);
            seen.outcome(&format!("{label}: read-only commit"), &db.commit(&sentinel_batch(1)));
        }
    }
}

/// The files of `image` by name, oldest first (MANIFEST, WALs, tables).
fn live_files(image: &DirImage) -> Vec<std::path::PathBuf> {
    image
        .files
        .keys()
        .filter(|p| p.file_name().is_some_and(|n| n != "LOCK") && p.extension().is_none_or(|e| e != "tmp"))
        .cloned()
        .collect()
}

/// A named change to a header.
type Patch = (&'static str, fn(&mut [u8]));

/// Rewrites a header field and reseals its CRC, so the damage is invisible
/// to the checksum and reaches the structural rules.
fn patch_header(image: &mut DirImage, file: &Path, patch: impl FnOnce(&mut [u8])) {
    let bytes = image.files.get_mut(file).unwrap();
    patch(&mut bytes[..HEADER_LEN]);
    assert!(reseal_header(&mut bytes[..HEADER_LEN]));
}

fn or_u64(h: &mut [u8], at: usize, bits: u64) {
    let v = u64::from_le_bytes(h[at..at + 8].try_into().unwrap()) | bits;
    h[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// Every kind of damage on `image`, each probed.
fn damage_sweep(seen: &mut Seen, name: &str, image: &DirImage, cfg: &Config) {
    probe(seen, &format!("{name}: undamaged"), image, cfg);
    let files = live_files(image);
    let total: u64 = files.iter().map(|p| image.files[p].len() as u64).sum();

    // Bit flips anywhere: headers, blocks, footers, WAL frames.
    let mut rng = Rng::new(0x1129);
    for n in 0..120 {
        let mut at = rng.below(total);
        let file = files
            .iter()
            .find(|p| {
                let len = image.files[*p].len() as u64;
                if at < len {
                    true
                } else {
                    at -= len;
                    false
                }
            })
            .unwrap();
        let mut damaged = image.clone();
        damaged.files.get_mut(file).unwrap()[at as usize] ^= 1 << rng.below(8);
        probe(seen, &format!("{name}: bit flip {n}"), &damaged, cfg);
    }

    for file in &files {
        let len = image.files[file].len();
        let fname = file.file_name().unwrap().to_string_lossy().into_owned();
        // Truncations.
        for cut in [0, 1, HEADER_LEN - 1, HEADER_LEN, len / 2, len - 1] {
            let mut damaged = image.clone();
            damaged.files.get_mut(file).unwrap().truncate(cut.min(len));
            probe(seen, &format!("{name}: {fname} cut to {cut}"), &damaged, cfg);
        }
        // A file removed: a table the MANIFEST names, the MANIFEST or a WAL
        // is missing (the listing catches it first, REC-5, REC-2).
        let mut damaged = image.clone();
        damaged.files.remove(file);
        probe(seen, &format!("{name}: {fname} is removed"), &damaged, cfg);
        // Header fields with a valid CRC: wrong file number, an unknown
        // ro_compat bit, an unknown incompat bit, a newer format version.
        let patches: [Patch; 4] = [
            ("file number", |h| or_u64(h, 56, 1 << 20)),
            ("ro_compat bit", |h| or_u64(h, 24, 1 << 3)),
            ("incompat bit", |h| or_u64(h, 32, 1 << 40)),
            ("format version", |h| h[8] = 2),
        ];
        for (what, patch) in patches {
            let mut damaged = image.clone();
            patch_header(&mut damaged, file, patch);
            probe(seen, &format!("{name}: {fname} {what}"), &damaged, cfg);
        }
    }

    // A table's bytes under another table's name, with a header that fits
    // the name: every checksum is valid, the MANIFEST disagrees.
    let ssts: Vec<_> = files.iter().filter(|p| p.extension().is_some_and(|e| e == "sst")).collect();
    if let [a, b, ..] = ssts[..] {
        let mut damaged = image.clone();
        let from = damaged.files[a].clone();
        let to_number = b.file_stem().unwrap().to_str().unwrap().parse::<u64>().unwrap();
        damaged.files.insert(b.clone(), from);
        patch_header(&mut damaged, b, |h| h[56..64].copy_from_slice(&to_number.to_le_bytes()));
        probe(seen, &format!("{name}: a table under another name"), &damaged, cfg);
    }
}

/// A table that is listed but gone when the open reads it (an outliving
/// snapshot's deferred deletion, RO-4): `Table::open` is the one that finds
/// out, not the listing check, and its message names the table.
fn vanishing_tables(seen: &mut Seen, image: &DirImage, cfg: &Config) {
    let tables: Vec<_> = live_files(image).into_iter().filter(|p| p.extension().is_some_and(|e| e == "sst")).collect();
    assert!(!tables.is_empty());
    for victim in tables {
        for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
            let disk = SimVfs::restore(image);
            let inner = disk.as_vfs();
            let gone = AtomicBool::new(false);
            let target = victim.clone();
            let remover = inner.clone();
            let vfs = HookVfs::wrap(inner, move |op| {
                if op.kind == HookOpKind::Open
                    && op.point == HookPoint::Before
                    && op.path == target
                    && !gone.swap(true, Ordering::SeqCst)
                {
                    remover.remove_file(op.path).unwrap();
                }
                HookAction::Proceed
            });
            let cfg = Config { access, ..cfg.clone() };
            let label = format!("vanishing table {} ({access:?})", victim.display());
            match open(&vfs, &cfg) {
                Err(e) => seen.error(&label, &e),
                Ok(db) => seen.outcome(&label, &db.close()),
            }
        }
    }
}

/// `Io` and `Poisoned`: the WAL refuses a write while a sentinel batch is
/// being committed, then the handle refuses everything.
fn failing_disk(seen: &mut Seen, cfg: &Config) {
    let disk = SimVfs::new(5);
    let armed = Arc::new(AtomicBool::new(false));
    let flag = armed.clone();
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        let wal = op.path.extension().is_some_and(|e| e == "wal");
        let writes = matches!(op.kind, HookOpKind::WriteAt | HookOpKind::SyncData);
        if flag.load(Ordering::SeqCst) && wal && writes && op.point == HookPoint::Before {
            HookAction::Fail(io::ErrorKind::StorageFull)
        } else {
            HookAction::Proceed
        }
    });
    let db = open(&vfs, cfg).unwrap();
    commit_range(&db, 0, 20);
    armed.store(true, Ordering::SeqCst);
    let mut failed = false;
    for i in 20..200 {
        if let Err(e) = db.commit(&sentinel_batch(i)) {
            seen.error("failing disk: commit", &e);
            failed = true;
            break;
        }
    }
    assert!(failed, "the injected write failure never reached a commit");
    seen.outcome("failing disk: commit after", &db.commit(&sentinel_batch(300)));
    seen.outcome("failing disk: snapshot after", &db.snapshot().map(|s| s.seq()));
    seen.outcome("failing disk: wait_idle", &db.hooks().wait_idle());
    observe(seen, "failing disk", &db);
    armed.store(false, Ordering::SeqCst);
    seen.outcome("failing disk: close", &db.close());
}

/// Runs the whole scenario on one configuration.
fn scenario(cfg: &Config) -> Seen {
    let mut seen = Seen::default();
    let disk = SimVfs::new(77);
    let vfs = disk.as_vfs();

    // Data: enough commits for several tables, and range deletions.
    let db = open(&vfs, cfg).unwrap();
    commit_range(&db, 0, 400);
    db.hooks().wait_idle().unwrap();
    observe(&mut seen, "live handle", &db);

    // InvalidArgument: limits and a backwards range, all naming sentinels.
    let mut too_long = KEY_TEXT.bytes.to_vec();
    too_long.extend([b'x'; 600]);
    let mut b = WriteBatch::new();
    b.put(too_long.clone(), value(&VAL_TEXT, 0));
    seen.outcome("invalid: long key", &db.commit(&b));
    let mut b = WriteBatch::new();
    b.put(key(&KEY_BIN, 0), [VAL_BIN.bytes, &vec![0u8; 2 << 20]].concat());
    seen.outcome("invalid: long value", &db.commit(&b));
    let mut b = WriteBatch::new();
    b.delete_range(key(&KEY_TEXT, 9), key(&KEY_TEXT, 1));
    seen.outcome("invalid: backwards range", &db.commit(&b));
    let mut b = WriteBatch::new();
    b.delete(too_long);
    seen.outcome("invalid: long delete key", &db.commit(&b));

    // Locked: a second handle, verify and inspect while it is open.
    seen.outcome("locked: open", &open(&vfs, cfg).map(drop));
    seen.outcome("locked: verify", &verify(vfs.clone(), Path::new(DIR), &VerifyOptions::default()));
    seen.outcome("locked: inspect", &inspect(vfs.clone(), Path::new(DIR)));
    seen.outcome("closed", &db.close());
    let closed = disk.dump();

    // A live WAL: more commits, no close.
    let db = open(&vfs, cfg).unwrap();
    observe(&mut seen, "reopened", &db);
    commit_range(&db, 400, 412);
    drop(db);
    let unclean = disk.dump();

    // The sentinels really are in the files: otherwise "no leak" proves nothing.
    for s in SENTINELS {
        let stored = [&closed, &unclean]
            .iter()
            .flat_map(|img| img.files.values())
            .any(|bytes| bytes.windows(s.bytes.len()).any(|w| w == s.bytes));
        assert!(stored, "{} is in no file of the directory", s.name);
    }

    damage_sweep(&mut seen, "closed", &closed, cfg);
    damage_sweep(&mut seen, "unclean", &unclean, cfg);
    vanishing_tables(&mut seen, &closed, cfg);
    failing_disk(&mut seen, cfg);
    seen
}

/// The scenario on `cfg`: all seven variants are produced and no text carries
/// a sentinel.
fn check_case(case: &str, cfg: &Config) {
    let seen = scenario(cfg);
    let missing: Vec<_> = VARIANTS.iter().filter(|v| !seen.variants.contains(*v)).collect();
    assert!(missing.is_empty(), "{case}: Error variants never produced: {missing:?}; saw {:?}", seen.variants);
    assert!(!seen.variants.contains("unknown"), "{case}: an Error variant this test does not know");
    assert!(seen.issues > 0, "{case}: verify reported no Issue, so the Issue path was not exercised");
    let leaks = seen.leaks();
    assert!(
        leaks.is_empty(),
        "{case}: {} of {} texts carry a sentinel (I-29); first ones:\n{}",
        leaks.len(),
        seen.texts.len(),
        leaks.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
}

// ------------------------------------------------------------- tests

#[test]
fn default_config_leaks_no_content() {
    check_case("default", &config_with(Durability::Sync, |_| {}));
}

#[test]
fn buffered_durability_leaks_no_content() {
    check_case("buffered", &config_with(Durability::Buffered, |_| {}));
}

#[test]
fn inline_profile_leaks_no_content() {
    check_case("kerno-inline", &config_with(Durability::Sync, Inline::adjust));
}

#[test]
fn m0_profile_leaks_no_content() {
    check_case("kerno-m0", &config_with(Durability::Sync, M0::adjust));
}

#[test]
fn serial_profile_leaks_no_content() {
    check_case("kerno-serial", &config_with(Durability::Sync, Serial::adjust));
}

/// The detector itself: every form of every sentinel is found, and text
/// without one is not. Without this, a broken needle would read as a pass.
#[test]
fn detector_finds_every_form_of_every_sentinel() {
    for s in SENTINELS {
        let owned = s.bytes.to_vec();
        let renderings = [
            format!("{owned:?}"),
            format!("{owned:#?}"),
            format!("{owned:x?}"),
            format!("{owned:X?}"),
            format!("{:?}", String::from_utf8_lossy(s.bytes)),
            s.bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            s.bytes.iter().map(|b| format!("{b:02X}")).collect::<String>(),
            s.bytes.iter().map(|b| format!("\\x{b:02x}")).collect::<String>(),
            s.bytes.iter().flat_map(|&b| std::ascii::escape_default(b)).map(char::from).collect::<String>(),
            // A longer key that starts with the sentinel, as the engine's keys do.
            format!("key {:?} is missing", key(s, 7)),
        ];
        for r in renderings {
            let wrapped = format!("corruption detected: 000007.sst: {r} (rule SST-O3)");
            assert!(!find_sentinels(&wrapped).is_empty(), "{}: not found in {wrapped:?}", s.name);
        }
    }
    let clean = "corruption detected: 000007.sst: referenced by the MANIFEST but missing at offset 4096, \
                 crc 0x1f2e3d4c, entries 55, rule SST-O3";
    assert!(find_sentinels(clean).is_empty());
}
