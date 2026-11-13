//! E9-02: public identifiers (ADR-0005 option 1, invariant I-27).
//!
//! * `issued_*`, `stable_*`: a token is issued with its unit, reads back, and
//!   is the same after a clean reopen and after later revisions.
//! * `mark_*`: the first token raises the layer mark to 3 and nothing lowers
//!   it; a reader of layer 2 refuses such a database.
//! * `deleting_*`, `retention_*`: deleting a unit removes both records in its
//!   transaction (its token then resolves like one never issued); retention of
//!   old revisions leaves the mapping of a live unit alone.
//! * `same_transaction_*`: the token records commit with the unit or not at
//!   all (a dropped change set, a failed sync and a synced commit followed by
//!   power loss).
//! * `tampered_*`: every way a text can be wrong is refused by the parser
//!   with its own error, and a well-formed token nobody holds resolves to
//!   nothing.
//! * `opaque_*`: the token carries nothing of the unit id or the creation
//!   order, and resolving it grants nothing.
//! * `legacy_*`: units without a token stay readable, and a directory
//!   written before this change still opens (committed fixture).
//! * `record_*`, `vectors_*`: the layout of the two stored records, the table
//!   of hex vectors of `docs/file-format/units-v3.md`, and the verifier.
//! * `export_*`, `import_*`: the token is part of the logical export and is
//!   the same after export and import; a file that gives one token to two
//!   units is refused with a typed error before the first write.
//! * `derived_*`: the same guarantees through `DerivedDb`.
//! * `fixture_*`: the committed directory with tokens
//!   (`tests/vectors/public-ids-v3/`).
//! * `stored::*` (feature `test-support`): damage written below the layer, and
//!   one test per cross-check of `verify_units` (UNIT-39 to UNIT-44).
//!
//! Every database test runs on `emulated_log` and on `kerno_db`, except
//! `legacy_directory_written_before_tokens_opens_and_verifies`: it opens a
//! committed directory of files, which only the engine with files can open
//! (`kerno_db`); `emulated_log` has no directory to open, and its legacy case
//! is `legacy_units_without_a_token_stay_readable`.
//!
//! Counts (`cargo test --release -p kerno-units --test public_ids`, as the
//! runner lists them): without the `test-support` feature 49 tests (48 run,
//! 1 ignored: `regenerate_fixture`, a by-hand regenerator of the committed
//! fixture), with it 77 (76 run, 1 ignored). Of those, 22 run on each engine
//! without the feature (44) and 36 with it (72); the other 5 are
//! engine-free: the text, the table of vectors, the fixture manifest, the
//! regenerator and the single-engine legacy-directory test.

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use harness::units::{Target, open_units, raw};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::sha256;
use kerno_testkit::vfs::{CrashKind, DirImage, OpKind, SimVfs};
use kerno_units::codec::{Key, LayerMeta, MAX_LAYER_VERSION};
use kerno_units::export::{EXPORT_VERSION, export};
use kerno_units::ids::{check_record, records};
use kerno_units::import::import;
use kerno_units::*;

type EmuLog = kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

/// Ids count up from `first`: a reopened database must start above the ids it holds.
fn cfg(first: u64) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(first)));
    c.engine = Options::small_for_tests();
    c
}

fn new_unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() }
}

fn open<E: Target>(disk: &SimVfs) -> UnitDb<E> {
    open_from::<E>(disk, 1)
}

fn open_from<E: Target>(disk: &SimVfs, first: u64) -> UnitDb<E> {
    open_units::<E>(disk.as_vfs(), &cfg(first)).expect("open")
}

fn issue<E: Target>(db: &UnitDb<E>, payload: &[u8]) -> (UnitId, PublicId) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let made = cs.create_public(new_unit(payload)).unwrap();
    cs.commit().unwrap();
    made
}

fn plain<E: Target>(db: &UnitDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let unit = cs.create(new_unit(payload)).unwrap();
    cs.commit().unwrap();
    unit
}

fn delete<E: Target>(db: &UnitDb<E>, unit: UnitId) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(unit, None).unwrap();
    cs.commit().unwrap();
}

fn clean<E: Target>(db: &UnitDb<E>) {
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
}

fn token_records<E: Target>(db: &UnitDb<E>) -> (usize, usize) {
    let all = raw(db);
    (all.iter().filter(|(k, _)| k[0] == 0x70).count(), all.iter().filter(|(k, _)| k[0] == 0x71).count())
}

fn mark_key() -> Vec<u8> {
    Key::LayerMeta { name: "layer".into() }.encode().unwrap()
}

fn mark_of<E: Target>(db: &UnitDb<E>) -> Vec<u8> {
    raw(db).into_iter().find(|(k, _)| *k == mark_key()).expect("layer mark").1
}

fn layer_version<E: Target>(db: &UnitDb<E>) -> u32 {
    LayerMeta::decode_upto(&mark_of(db), MAX_LAYER_VERSION).unwrap().layer_version
}

// ------------------------------------------------------------ issue and read

fn issued_token_reads_back_and_resolves<E: Target>() {
    let disk = SimVfs::new(1);
    let db = open::<E>(&disk);
    let (unit, token) = issue(&db, b"one");
    let (other, other_token) = issue(&db, b"two");
    assert_ne!(token, other_token);
    assert_eq!(db.public_id(unit).unwrap(), Some(token));
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(unit));
    assert_eq!(db.resolve_public_id(&other_token).unwrap(), Some(other));
    // The text form is the documented one and parses back to the same token.
    let text = token.to_string();
    assert_eq!((text.len(), &text[..3]), (29, "k1_"));
    assert_eq!(text.parse::<PublicId>(), Ok(token));
    clean(&db);
}

fn stable_after_reopen_and_later_revisions<E: Target>() {
    let disk = SimVfs::new(2);
    let db = open::<E>(&disk);
    let (unit, token) = issue(&db, b"v1");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(unit, UnitUpdate { payload: b"v2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(unit, None).unwrap();
    cs.commit().unwrap();
    db.close().unwrap();

    let db = open_from::<E>(&disk, 100);
    assert_eq!(db.public_id(unit).unwrap(), Some(token));
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(unit));
    // A unit created after the reopen gets a token of its own.
    let (_, fresh) = issue(&db, b"v3");
    assert_ne!(fresh, token);
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(unit));
    clean(&db);
}

// ------------------------------------------------------------- layer mark

fn mark_is_raised_to_3_by_the_first_token_and_never_lowered<E: Target>() {
    let disk = SimVfs::new(16);
    let db = open::<E>(&disk);
    let unit = plain(&db, b"before");
    assert_eq!(layer_version(&db), 1, "a database without tokens stays at layer 1");
    let (with_token, _) = issue(&db, b"token");
    assert_eq!(layer_version(&db), 3);
    // Neither deleting the only token nor a policy change lowers it.
    delete(&db, with_token);
    assert_eq!(token_records(&db), (0, 0));
    db.set_policy("doc", HistoryPolicy::Versioned, ChangeMeta::default()).unwrap();
    plain(&db, b"after");
    assert_eq!(layer_version(&db), 3);
    assert!(db.get_unit(unit).unwrap().is_some());
    clean(&db);
}

fn layer2_reader_refuses_a_database_with_tokens<E: Target>() {
    let disk = SimVfs::new(17);
    let db = open::<E>(&disk);
    issue(&db, b"x");
    let mark = mark_of(&db);
    // A build that knows layers 1 and 2 reads the mark first and refuses it.
    for reader in [1, 2] {
        let e = LayerMeta::decode_upto(&mark, reader).unwrap_err();
        assert_eq!((e.rule, e.found_layer_version), (UnitRule::Unit16, Some(3)), "reader {reader}");
        assert_eq!(e.class(), ErrorClass::IncompatibleLayer);
        assert!(
            matches!(UnitError::from(e), UnitError::IncompatibleLayer { found: 3, supported } if supported == reader)
        );
    }
    assert_eq!(LayerMeta::decode_upto(&mark, MAX_LAYER_VERSION).unwrap().layer_version, 3);
}

// -------------------------------------------------------------- lifecycle

fn deleting_a_unit_removes_its_token_like_one_never_issued<E: Target>() {
    let disk = SimVfs::new(3);
    let db = open::<E>(&disk);
    let (unit, token) = issue(&db, b"secret");
    let (kept, kept_token) = issue(&db, b"kept");
    assert_eq!(token_records(&db), (2, 2));
    delete(&db, unit);
    // The token resolves exactly like one that was never issued.
    let never: PublicId = GOOD.parse().unwrap();
    assert_eq!(db.resolve_public_id(&token).unwrap(), None);
    assert_eq!(db.resolve_public_id(&never).unwrap(), None);
    assert_eq!(db.public_id(unit).unwrap(), None);
    // The other unit's mapping is untouched, and only its records remain.
    assert_eq!(db.resolve_public_id(&kept_token).unwrap(), Some(kept));
    assert_eq!(token_records(&db), (1, 1));
    // The unit itself is still the layer's to report.
    let u = db.get_unit(unit).unwrap().unwrap();
    assert_eq!((u.lifecycle, u.payload), (Lifecycle::Deleted, None));
    clean(&db);
    // Stays so after a reopen.
    db.close().unwrap();
    let db = open_from::<E>(&disk, 100);
    assert_eq!(db.resolve_public_id(&token).unwrap(), None);
    assert_eq!(token_records(&db), (1, 1));
    clean(&db);
}

fn retention_leaves_the_mapping_of_a_live_unit<E: Target>() {
    let disk = SimVfs::new(19);
    let db = open::<E>(&disk);
    let (unit, token) = issue(&db, b"r1");
    for i in 2..=5u8 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(unit, UnitUpdate { payload: vec![b'r', i], schema_version: None, metadata: None }, None).unwrap();
        cs.commit().unwrap();
    }
    let report =
        db.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default()).unwrap();
    assert!(report.revisions_pruned >= 1, "retention pruned nothing: {report:?}");
    assert_eq!(db.get_unit(unit).unwrap().unwrap().oldest.0, 5);
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(unit));
    assert_eq!(db.public_id(unit).unwrap(), Some(token));
    assert_eq!(token_records(&db), (1, 1));
    clean(&db);
}

// ------------------------------------------------------- one transaction

fn same_transaction_a_dropped_change_set_leaves_nothing<E: Target>() {
    let disk = SimVfs::new(4);
    let db = open::<E>(&disk);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let (unit, token) = cs.create_public(new_unit(b"x")).unwrap();
    cs.abort();
    assert_eq!(db.public_id(unit).unwrap(), None);
    assert_eq!(db.resolve_public_id(&token).unwrap(), None);
    assert!(db.get_unit(unit).unwrap().is_none());
    assert!(raw(&db).iter().all(|(k, _)| k[0] < 0x70), "a token record outlived its change set");
}

/// The commit's data sync fails and the power goes: the unit and its two
/// records are all there or none are. Which of the two happens is the
/// engine's to decide (an `emulated_log` commit that wrote nothing durable
/// leaves nothing; `kerno_db` may have the frame on disk), so the test fails
/// the 1st and then the 2nd data sync counted from the arming of the fault
/// (not necessarily the commit's own syncs: whichever the engine issues
/// first) and asserts the invariant on whatever the image holds.
fn same_transaction_a_failed_sync_leaves_neither_unit_nor_token<E: Target>() {
    for nth in 1..=2u64 {
        let disk = SimVfs::new(5 + nth);
        let db = open::<E>(&disk);
        let (kept, kept_token) = issue(&db, b"kept");
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let (unit, token) = cs.create_public(new_unit(b"lost")).unwrap();
        disk.fail_when(|op| op.kind == OpKind::SyncData, nth, None);
        let result = cs.commit();
        if !disk.fault_injected() {
            // The commit needed fewer syncs than `nth`: it is a plain commit.
            assert!(result.is_ok());
            continue;
        }
        assert!(result.is_err());
        let image = disk.crash_now(CrashKind::PowerLoss);
        let after = open::<E>(&image);
        assert_eq!(after.resolve_public_id(&kept_token).unwrap(), Some(kept));
        // The commit is all there or not at all (I-21): both halves, or neither.
        let unit_there = after.get_unit(unit).unwrap().is_some();
        assert_eq!(after.public_id(unit).unwrap().is_some(), unit_there, "nth {nth}");
        assert_eq!(after.resolve_public_id(&token).unwrap().is_some(), unit_there, "nth {nth}");
        assert_eq!(token_records(&after), if unit_there { (2, 2) } else { (1, 1) }, "nth {nth}");
        clean(&after);
    }
}

/// The other branch: the commit is acknowledged and durable, then the power
/// goes. The unit and both records are all there.
fn same_transaction_a_synced_commit_survives_power_loss_whole<E: Target>() {
    let disk = SimVfs::new(20);
    let db = open::<E>(&disk);
    let (unit, token) = issue(&db, b"durable");
    let image = disk.crash_now(CrashKind::PowerLoss);
    let after = open_from::<E>(&image, 100);
    assert!(after.get_unit(unit).unwrap().is_some());
    assert_eq!(after.public_id(unit).unwrap(), Some(token));
    assert_eq!(after.resolve_public_id(&token).unwrap(), Some(unit));
    assert_eq!(token_records(&after), (1, 1));
    clean(&after);
}

// ------------------------------------------------------------ tampered tokens

/// The token of the 16 bytes `0, 1, ..., 15`.
const GOOD: &str = "k1_aaaqeayeaudaocajbifqydiob4";

fn good() -> String {
    GOOD.to_owned()
}

#[test]
fn tampered_text_is_refused_with_its_own_error() {
    let ok = good();
    let cases: Vec<(String, PublicIdError)> = vec![
        (String::new(), PublicIdError::Prefix),
        ("k1".into(), PublicIdError::Prefix),
        ("k1aaaqeayeaudaocajbifqydiob4".into(), PublicIdError::Prefix),
        (ok.replacen('k', "x", 1), PublicIdError::Prefix),
        (ok.replacen('k', "K", 1), PublicIdError::Prefix),
        (ok.replacen('_', "-", 1), PublicIdError::Prefix),
        (ok.replacen("k1_", "k_", 1), PublicIdError::Prefix),
        (ok.replacen("k1_", "ka_", 1), PublicIdError::Prefix),
        (ok.replacen("k1_", "k2_", 1), PublicIdError::Version { found: 2 }),
        (ok.replacen("k1_", "k0_", 1), PublicIdError::Version { found: 0 }),
        (ok.replacen("k1_", "k01_", 1), PublicIdError::Prefix),
        (ok.replacen("k1_", "k99999999999_", 1), PublicIdError::Version { found: u32::MAX }),
        (ok[..ok.len() - 1].to_owned(), PublicIdError::Length { found: 25 }),
        (format!("{ok}a"), PublicIdError::Length { found: 27 }),
        ("k1_".into(), PublicIdError::Length { found: 0 }),
        (ok.to_uppercase().replacen('K', "k", 1), PublicIdError::Alphabet { at: 3 }),
        (format!("{} ", &ok[..ok.len() - 1]), PublicIdError::Alphabet { at: 28 }),
        (ok.replacen("aaaq", "aa0q", 1), PublicIdError::Alphabet { at: 5 }),
        (ok.replacen("aaaq", "aa1q", 1), PublicIdError::Alphabet { at: 5 }),
        (ok.replacen("aaaq", "aa8q", 1), PublicIdError::Alphabet { at: 5 }),
        (ok.replacen("aaaq", "aa=q", 1), PublicIdError::Alphabet { at: 5 }),
        // 'b' as the last character sets a spare bit: a second text for the same id.
        (format!("{}b", &ok[..ok.len() - 1]), PublicIdError::NonCanonical),
    ];
    for (text, want) in cases {
        assert_eq!(text.parse::<PublicId>(), Err(want), "{text:?}");
    }
    assert!(ok.parse::<PublicId>().is_ok());
}

fn tampered_but_well_formed_tokens_name_nothing<E: Target>() {
    let disk = SimVfs::new(6);
    let db = open::<E>(&disk);
    let (_, token) = issue(&db, b"a");
    let text = token.to_string();
    // Flip each body character in turn: still a token, never an issued one.
    for i in 3..text.len() - 1 {
        let mut bytes = text.clone().into_bytes();
        bytes[i] = if bytes[i] == b'a' { b'b' } else { b'a' };
        let flipped: PublicId = String::from_utf8(bytes).unwrap().parse().unwrap();
        assert_ne!(flipped, token);
        assert_eq!(db.resolve_public_id(&flipped).unwrap(), None, "position {i}");
    }
    // A token of another database names nothing here.
    let elsewhere = SimVfs::new(7);
    let other = open::<E>(&elsewhere);
    let (_, foreign) = issue(&other, b"b");
    assert_eq!(db.resolve_public_id(&foreign).unwrap(), None);
    // A syntactically valid token that was never issued.
    let never: PublicId = good().parse().unwrap();
    assert_eq!(db.resolve_public_id(&never).unwrap(), None);
}

// ----------------------------------------------------- opacity, no authority

fn opaque_tokens_follow_neither_id_nor_creation_order<E: Target>() {
    let disk = SimVfs::new(8);
    let db = open::<E>(&disk);
    // The ids are 1, 2, 3, ... in creation order (CounterIds).
    let made: Vec<(UnitId, PublicId)> = (0..64u8).map(|i| issue(&db, &[i])).collect();
    // Sorted by the value the text encodes (`2`..`7` come after `z`), not by ASCII.
    let rank = |t: &String| -> Vec<u8> { t.bytes().map(|b| if b.is_ascii_digit() { b + 100 } else { b }).collect() };
    let texts: Vec<String> = made.iter().map(|(_, t)| t.to_string()).collect();
    let mut sorted = texts.clone();
    sorted.sort_by_key(rank);
    let mut reversed = sorted.clone();
    reversed.reverse();
    assert_ne!(texts, sorted, "tokens sort in creation order");
    assert_ne!(texts, reversed, "tokens sort against creation order");
    assert_eq!(texts.iter().collect::<BTreeSet<_>>().len(), 64);
    // Not a function of the id: a second database that mints the same ids,
    // at the same fixed time, issues other tokens.
    let twin = open::<E>(&SimVfs::new(15));
    for (i, (unit, token)) in made.iter().enumerate().take(8) {
        let (twin_unit, twin_token) = issue(&twin, &[i as u8]);
        assert_eq!(twin_unit, *unit);
        assert_ne!(twin_token, *token, "same id, same token");
    }
}

// ----------------------------------------------------------------- legacy

fn legacy_units_without_a_token_stay_readable<E: Target>() {
    let disk = SimVfs::new(9);
    let db = open::<E>(&disk);
    // A plain create writes what a build before this change wrote.
    let old = plain(&db, b"old");
    assert_eq!(db.public_id(old).unwrap(), None);
    assert_eq!(db.get_unit(old).unwrap().unwrap().payload.as_deref(), Some(&b"old"[..]));
    clean(&db);
    // An export of units without tokens imports without tokens.
    let src = open::<E>(&SimVfs::new(10));
    let carried = plain(&src, b"carried");
    let target = open::<E>(&SimVfs::new(11));
    let report = import(&target, &export(&src).unwrap()).unwrap();
    assert_eq!(report.tokens, 0);
    assert_eq!(target.public_id(carried).unwrap(), None);
    assert_eq!(layer_version(&target), 1, "no token, no layer 3");
    clean(&target);
    db.close().unwrap();
    // Reopened, mixed with a unit that has one.
    let db = open_from::<E>(&disk, 100);
    let (_, token) = issue(&db, b"new");
    assert_eq!(db.public_id(old).unwrap(), None);
    assert!(db.resolve_public_id(&token).unwrap().is_some());
    clean(&db);
}

fn committed_files() -> BTreeMap<String, Vec<u8>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/derived-v2/db-with-vector");
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name().into_string().unwrap(), std::fs::read(e.path()).unwrap())
        })
        .collect()
}

/// The directory E9-01 committed was written before this change: it has a
/// unit and no token. Only `kerno_db` has directories of files to open; the
/// `emulated_log` engine keeps its state in memory and cannot open one, so
/// this test has no `emulated_log` twin (its legacy case is
/// `legacy_units_without_a_token_stay_readable`).
#[test]
fn legacy_directory_written_before_tokens_opens_and_verifies() {
    let files = committed_files();
    let image = DirImage {
        dirs: BTreeSet::from([PathBuf::from("/db")]),
        files: files.iter().map(|(n, b)| (Path::new("/db").join(n), b.clone())).collect(),
        unlinked_open: Vec::new(),
    };
    let disk = SimVfs::restore(&image);
    let db = open_units::<KernoDb>(disk.as_vfs(), &cfg(100)).unwrap();
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&1u64.to_be_bytes());
    let unit = UnitId(id);
    assert!(db.get_unit(unit).unwrap().is_some());
    assert_eq!(db.public_id(unit).unwrap(), None);
    assert_eq!(layer_version(&db), 2);
    clean(&db);
    // New units on top of it get tokens (the mark goes 2 to 3); the old one still has none.
    let (fresh, token) = issue(&db, b"after");
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(fresh));
    assert_eq!(db.public_id(unit).unwrap(), None);
    assert_eq!(layer_version(&db), 3);
    clean(&db);
}

// ----------------------------------------------------------------- records

/// The stored layout, byte for byte: tag, 16 bytes, then the envelope
/// `01 00` and 16 bytes.
fn record_layout_is_pinned<E: Target>() {
    let disk = SimVfs::new(12);
    let db = open::<E>(&disk);
    let (unit, token) = issue(&db, b"p");
    let text = token.to_string();
    let store: BTreeMap<Vec<u8>, Vec<u8>> = raw(&db).into_iter().collect();
    let by_token: Vec<_> = store.iter().filter(|(k, _)| k[0] == 0x70).collect();
    let by_unit: Vec<_> = store.iter().filter(|(k, _)| k[0] == 0x71).collect();
    assert_eq!((by_token.len(), by_unit.len()), (1, 1));
    let (tk, tv) = by_token[0];
    let (uk, uv) = by_unit[0];
    assert_eq!((tk.len(), tv.len(), uk.len(), uv.len()), (17, 18, 17, 18));
    assert_eq!(tv[..2], [PUBLIC_ID_RECORD_VERSION, 0]);
    assert_eq!(uv[..2], [PUBLIC_ID_RECORD_VERSION, 0]);
    assert_eq!(PUBLIC_ID_RECORD_VERSION, 1);
    assert_eq!(&tv[2..], &unit.0);
    assert_eq!(&uk[1..], &unit.0);
    assert_eq!(&tk[1..], &uv[2..], "the two records name the same token");
    // The key bytes are the 128 bits the text encodes (RFC 4648, lower case).
    assert_eq!(text, format!("k1_{}", base32(&tk[1..])));
    // The production builder writes the same bytes.
    let [(k1, v1), (k2, v2)] = records(unit, &token);
    assert_eq!((&k1, &v1), (tk, tv));
    assert_eq!((&k2, &v2), (uk, uv));
}

/// RFC 4648 base32, lower case, no padding.
fn base32(bytes: &[u8]) -> String {
    let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
    let bits: String = bytes.iter().map(|b| format!("{b:08b}")).collect();
    let padded = format!("{bits}{}", "0".repeat((5 - bits.len() % 5) % 5));
    padded
        .as_bytes()
        .chunks(5)
        .map(|c| alphabet[usize::from_str_radix(std::str::from_utf8(c).unwrap(), 2).unwrap()] as char)
        .collect()
}

fn record_verifier_accepts_tokens<E: Target>() {
    let disk = SimVfs::new(13);
    let db = open::<E>(&disk);
    for i in 0..5u8 {
        issue(&db, &[i]);
    }
    plain(&db, b"no token");
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
    assert_eq!(report.counts.public_id_records, 10);
}

// ---------------------------------------------------- vectors of the document

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex {s:?}");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// The cells of a table row `| a | b | c | d |`, with their backticks removed.
fn cells(line: &str) -> Option<Vec<String>> {
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|c| c.trim().trim_matches('`').to_owned()).collect())
}

/// Each row of the table of `docs/file-format/units-v3.md` section 6 is built
/// or decoded with the production codec and gives the stated result.
#[test]
fn vectors_of_the_document_match_the_codec_byte_for_byte() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/file-format/units-v3.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let unit = UnitId(unhex("0102030405060708090a0b0c0d0e0f10").try_into().unwrap());
    let token: PublicId = GOOD.parse().unwrap();
    let [(token_key, token_value), (reverse_key, reverse_value)] = records(unit, &token);
    let (mut shape_ok, mut shape_bad, mut marks_ok, mut marks_bad) = (0, 0, 0, 0);
    for line in text.lines() {
        let Some(c) = cells(line) else { continue };
        if c.len() != 4 || !c[0].chars().next().is_some_and(|ch| ch.is_ascii_lowercase()) || c[0] == "nombre" {
            continue;
        }
        let (name, result) = (c[0].as_str(), c[3].as_str());
        if let Some(rest) = name.strip_prefix("mark_") {
            // `| mark_… | bytes | reader | result |`
            let reader: u32 = c[2].parse().unwrap_or_else(|_| panic!("{name}: reader {:?}", c[2]));
            let got = LayerMeta::decode_upto(&unhex(&c[1]), reader);
            match (result, got) {
                ("ok", Ok(m)) => {
                    assert_eq!(m.layer_version, 3, "{name}");
                    assert_eq!(m.encode().unwrap(), unhex(&c[1]), "{name}: re-encoded bytes");
                    marks_ok += 1;
                }
                (rule, Err(e)) => {
                    assert_eq!(e.rule.id(), rule, "{name} ({rest})");
                    marks_bad += 1;
                }
                (rule, Ok(_)) => panic!("{name}: decoded, wanted {rule}"),
            }
            continue;
        }
        let (key, value) = (unhex(&c[1]), unhex(&c[2]));
        match (result, check_record(&key, &value)) {
            ("ok", Ok(other)) => {
                let want: &(Vec<u8>, Vec<u8>) = if key[0] == 0x70 {
                    &(token_key.clone(), token_value.clone())
                } else {
                    &(reverse_key.clone(), reverse_value.clone())
                };
                assert_eq!((&key, &value), (&want.0, &want.1), "{name}: bytes the codec writes");
                assert_eq!(other.len(), 16);
                shape_ok += 1;
            }
            (rule, Err(e)) => {
                assert_eq!(e.rule.id(), rule, "{name}: {e}");
                shape_bad += 1;
            }
            (rule, Ok(_)) => panic!("{name}: accepted, wanted {rule}"),
        }
    }
    // A vector lost or added without review breaks the count.
    assert_eq!((shape_ok, shape_bad, marks_ok, marks_bad), (2, 15, 1, 3));
}

// ------------------------------------------------------------ export, import

fn export_carries_tokens_and_import_restores_them<E: Target>() {
    let src = open::<E>(&SimVfs::new(21));
    let (a, ta) = issue(&src, b"a");
    let untokened = plain(&src, b"plain");
    let (b, tb) = issue(&src, b"b");
    let (gone, tgone) = issue(&src, b"gone");
    delete(&src, gone);
    // A revision more, so the first revision of `b` is not the head.
    let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(b, UnitUpdate { payload: b"b2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    let bytes = export(&src).unwrap();

    // Ids count up from 100 so a unit made after the import cannot reuse an imported id.
    let target = open_from::<E>(&SimVfs::new(22), 100);
    let report = import(&target, &bytes).unwrap();
    assert_eq!(report.tokens, 2);
    // A token is stable across export and import.
    assert_eq!(target.public_id(a).unwrap(), Some(ta));
    assert_eq!(target.public_id(b).unwrap(), Some(tb));
    assert_eq!(target.resolve_public_id(&ta).unwrap(), Some(a));
    assert_eq!(target.resolve_public_id(&tb).unwrap(), Some(b));
    assert_eq!(target.public_id(untokened).unwrap(), None);
    assert_eq!(target.resolve_public_id(&tgone).unwrap(), None);
    assert_eq!(token_records(&target), (2, 2));
    assert_eq!(layer_version(&target), 3);
    clean(&target);
    // The exported bytes are the same again, and a unit made afterwards gets
    // a token that collides with none of them.
    assert_eq!(export(&target).unwrap(), bytes);
    let mut cs = target.begin_changeset(ChangeMeta::default()).unwrap();
    let (_, fresh) = cs.create_public(new_unit(b"after import")).unwrap();
    cs.commit().unwrap();
    assert!(![ta, tb, tgone].contains(&fresh));
    clean(&target);
}

fn export_import_export_keeps_every_token_across_two_hops<E: Target>() {
    let src = open::<E>(&SimVfs::new(23));
    let made: Vec<(UnitId, PublicId)> = (0..6u8).map(|i| issue(&src, &[i])).collect();
    let hop1 = open::<E>(&SimVfs::new(24));
    import(&hop1, &export(&src).unwrap()).unwrap();
    let hop2 = open::<E>(&SimVfs::new(25));
    import(&hop2, &export(&hop1).unwrap()).unwrap();
    for (unit, token) in made {
        assert_eq!(hop2.public_id(unit).unwrap(), Some(token));
        assert_eq!(hop2.resolve_public_id(&token).unwrap(), Some(unit));
    }
    assert_eq!(export(&hop2).unwrap(), export(&src).unwrap());
}

/// An export with its body changed by `f` and its digest recomputed, so that
/// only the rule under test can refuse it.
fn retouched(bytes: &[u8], f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut body = bytes[..bytes.len() - 32].to_vec();
    f(&mut body);
    let digest = *blake3::hash(&body).as_bytes();
    body.extend_from_slice(&digest);
    body
}

/// The offset of record `i` of the token section: after the 16-byte header,
/// the default policy byte, the policy count (no policies in these sources)
/// and the token count.
fn token_at(body: &[u8], n: usize, i: usize) -> usize {
    assert_eq!(body[17..21], [0; 4], "the sources have no per-type policy");
    assert_eq!(body[21..25], (n as u32).to_be_bytes(), "token count");
    25 + i * 32
}

fn assert_refused_before_any_write<E: Target>(target: &UnitDb<E>, e: &UnitError, what: &str) {
    // The refusal is raised while the file is read: the target has no commit.
    assert_eq!(target.snapshot().unwrap().seq().0, 0, "{what}: {e}");
    assert!(raw(target).is_empty(), "{what}: the target was written");
}

fn import_refuses_one_token_for_two_units<E: Target>() {
    let src = open::<E>(&SimVfs::new(26));
    issue(&src, b"a");
    issue(&src, b"b");
    let bytes = export(&src).unwrap();
    let dup = retouched(&bytes, |body| {
        let (first, second) = (token_at(body, 2, 0), token_at(body, 2, 1));
        let token: Vec<u8> = body[first + 16..first + 32].to_vec();
        body[second + 16..second + 32].copy_from_slice(&token);
    });
    let target = open::<E>(&SimVfs::new(27));
    let e = import(&target, &dup).unwrap_err();
    assert!(matches!(e, UnitError::PublicIdTaken), "{e:?}");
    assert_refused_before_any_write(&target, &e, "duplicate token");
    // The text of the error names neither a token nor a unit.
    let text = e.to_string();
    assert!(!text.contains("k1_"), "{text}");
    // The unmodified file still imports.
    import(&target, &bytes).unwrap();
}

fn import_refuses_token_records_that_no_writer_produces<E: Target>() {
    let src = open::<E>(&SimVfs::new(28));
    issue(&src, b"a");
    issue(&src, b"b");
    plain(&src, b"c");
    let d = plain(&src, b"d");
    delete(&src, d);
    let bytes = export(&src).unwrap();
    let corruption = |e: &UnitError| matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_)));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "unit not in the file",
            retouched(&bytes, |b| {
                let at = token_at(b, 2, 1);
                b[at..at + 16].copy_from_slice(&[0xff; 16]);
            }),
        ),
        (
            "constant token",
            retouched(&bytes, |b| {
                let at = token_at(b, 2, 1);
                b[at + 16..at + 32].copy_from_slice(&[7; 16]);
            }),
        ),
        (
            "ids not increasing",
            retouched(&bytes, |b| {
                let (x, y) = (token_at(b, 2, 0), token_at(b, 2, 1));
                let first: Vec<u8> = b[x..x + 16].to_vec();
                let second: Vec<u8> = b[y..y + 16].to_vec();
                b[x..x + 16].copy_from_slice(&second);
                b[y..y + 16].copy_from_slice(&first);
            }),
        ),
        (
            "deleted unit",
            retouched(&bytes, |b| {
                let at = token_at(b, 2, 1);
                b[at..at + 16].copy_from_slice(&d.0);
            }),
        ),
        (
            "truncated section",
            retouched(&bytes, |b| {
                b.truncate(b.len() - 16);
            }),
        ),
        (
            "bytes after the section",
            retouched(&bytes, |b| {
                b.push(0);
            }),
        ),
    ];
    for (what, file) in cases {
        let target = open::<E>(&SimVfs::new(29));
        let e = import(&target, &file).unwrap_err();
        assert!(corruption(&e), "{what}: {e:?}");
        assert_refused_before_any_write(&target, &e, what);
    }
}

fn import_of_a_version_0_export_is_refused<E: Target>() {
    assert_eq!(EXPORT_VERSION, 1, "the token section raised the draft's format_version");
    let src = open::<E>(&SimVfs::new(30));
    issue(&src, b"a");
    let bytes = export(&src).unwrap();
    assert_eq!(bytes[8..12], 1u32.to_be_bytes());
    let old = retouched(&bytes, |b| b[8..12].copy_from_slice(&0u32.to_be_bytes()));
    let target = open::<E>(&SimVfs::new(31));
    let e = import(&target, &old).unwrap_err();
    assert!(matches!(e, UnitError::Storage(kerno_core::Error::Incompatible(_))), "{e:?}");
    assert_refused_before_any_write(&target, &e, "version 0");
}

// ------------------------------------------------------------ DerivedDb

fn derived_db_issues_resolves_and_forgets_tokens<E: Target>() {
    let (dd, _) = DerivedDb::open_persisted(open::<E>(&SimVfs::new(32))).unwrap();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let (unit, token) = cs.create_public(new_unit(b"d")).unwrap();
    let plain_unit = cs.create(new_unit(b"plain")).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.public_id(unit).unwrap(), Some(token));
    assert_eq!(dd.resolve_public_id(&token).unwrap(), Some(unit));
    assert_eq!(dd.public_id(plain_unit).unwrap(), None);
    // The same snapshot answers the same.
    assert_eq!(dd.snapshot().unwrap().resolve_public_id(&token).unwrap(), Some(unit));
    // A dropped change set leaves nothing.
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let (ghost, ghost_token) = cs.create_public(new_unit(b"x")).unwrap();
    cs.abort();
    assert_eq!(dd.resolve_public_id(&ghost_token).unwrap(), None);
    assert_eq!(dd.public_id(ghost).unwrap(), None);
    // Deleting through the derived layer removes the mapping.
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(unit, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.resolve_public_id(&token).unwrap(), None);
    assert_eq!(dd.public_id(unit).unwrap(), None);
    let report = dd.snapshot().unwrap().verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
}

// ------------------------------------------------- the committed directory

const FIXTURE_DIR: &str = "tests/vectors/public-ids-v3/db-with-token";
const FIXTURE_MANIFEST: &str = "tests/vectors/public-ids-v3/MANIFEST";
const FIXTURE_TOKEN: &str = "tests/vectors/public-ids-v3/TOKEN";
const FIXTURE_SEED: u64 = 33;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The fixture content: unit 1 with a token, unit 2 without, unit 3 with a
/// token and then deleted. Returns the token of unit 1.
fn build_fixture<E: Target>(disk: &SimVfs) -> PublicId {
    let db = open_units::<E>(disk.as_vfs(), &cfg(1)).unwrap();
    let (_, token) = issue(&db, b"hello");
    plain(&db, b"plain");
    let (third, _) = issue(&db, b"gone");
    delete(&db, third);
    db.close().unwrap();
    token
}

fn fixture_files() -> BTreeMap<String, Vec<u8>> {
    let dir = root().join(FIXTURE_DIR);
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| {
            let e = e.unwrap();
            (e.file_name().into_string().unwrap(), std::fs::read(e.path()).unwrap())
        })
        .collect()
}

fn manifest_of(files: &BTreeMap<String, Vec<u8>>) -> String {
    files.iter().map(|(n, b)| format!("{}  {}  {n}\n", sha256::hex(&sha256::digest(b)), b.len())).collect()
}

fn written_files(disk: &SimVfs) -> BTreeMap<String, Vec<u8>> {
    disk.dump()
        .files
        .into_iter()
        .filter_map(|(p, b)| {
            let name = p.file_name()?.to_str()?.to_owned();
            (name != "LOCK").then_some((name, b))
        })
        .collect()
}

/// Regenerates the committed directory after a deliberate layout change:
/// `cargo test --release -p kerno-units --test public_ids -- --ignored regenerate_fixture`.
#[test]
#[ignore = "writes the committed fixture; run by hand after a deliberate layout change"]
fn regenerate_fixture() {
    let disk = SimVfs::new(FIXTURE_SEED);
    let token = build_fixture::<KernoDb>(&disk);
    let files = written_files(&disk);
    let dir = root().join(FIXTURE_DIR);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    std::fs::write(root().join(FIXTURE_MANIFEST), manifest_of(&files)).unwrap();
    std::fs::write(root().join(FIXTURE_TOKEN), format!("{token}\n")).unwrap();
}

#[test]
fn fixture_manifest_lists_the_committed_files() {
    let files = fixture_files();
    assert!(!files.is_empty(), "the fixture directory is empty");
    let want = std::fs::read_to_string(root().join(FIXTURE_MANIFEST)).expect("MANIFEST");
    assert_eq!(manifest_of(&files), want, "a file of {FIXTURE_DIR} differs from MANIFEST");
}

/// The disk and the token to expect: the committed directory for the engine
/// that has files, the same content built in memory for the other (whose
/// token is its own, so only its shape and round trip are checked).
fn fixture_disk<E: Target>() -> (SimVfs, Option<PublicId>) {
    if E::NAME == <KernoDb as Target>::NAME {
        let image = DirImage {
            dirs: BTreeSet::from([PathBuf::from("/db")]),
            files: fixture_files().into_iter().map(|(n, b)| (Path::new("/db").join(n), b)).collect(),
            unlinked_open: Vec::new(),
        };
        let text = std::fs::read_to_string(root().join(FIXTURE_TOKEN)).expect("TOKEN");
        (SimVfs::restore(&image), Some(text.trim().parse().expect("TOKEN holds a token")))
    } else {
        let disk = SimVfs::new(FIXTURE_SEED);
        build_fixture::<E>(&disk);
        (disk, None)
    }
}

fn fixture_directory_opens_resolves_and_verifies<E: Target>() {
    let (disk, pinned) = fixture_disk::<E>();
    let db = open_units::<E>(disk.as_vfs(), &cfg(100)).unwrap();
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&1u64.to_be_bytes());
    let first = UnitId(id);
    id[8..].copy_from_slice(&2u64.to_be_bytes());
    let second = UnitId(id);
    id[8..].copy_from_slice(&3u64.to_be_bytes());
    let third = UnitId(id);
    let token = db.public_id(first).unwrap().expect("unit 1 has a token");
    if let Some(pinned) = pinned {
        assert_eq!(token, pinned, "the token the directory holds is the one TOKEN records");
    }
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(first));
    assert_eq!(db.public_id(second).unwrap(), None);
    assert_eq!(db.public_id(third).unwrap(), None, "the deleted unit's mapping was removed");
    assert_eq!(token_records(&db), (1, 1));
    assert_eq!(layer_version(&db), 3);
    clean(&db);
    // A unit made on top of the opened directory gets its own token.
    let (_, fresh) = issue(&db, b"on top");
    assert_ne!(fresh, token);
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(first));
    clean(&db);
}

fn fixture_directory_is_refused_by_a_layer2_reader<E: Target>() {
    let (disk, _) = fixture_disk::<E>();
    let db = open_units::<E>(disk.as_vfs(), &cfg(100)).unwrap();
    let mark = mark_of(&db);
    let e = LayerMeta::decode_upto(&mark, 2).unwrap_err();
    assert_eq!((e.rule, e.found_layer_version), (UnitRule::Unit16, Some(3)));
}

/// Tampered stored records, written under the feature that opens the raw path.
#[cfg(feature = "test-support")]
mod stored {
    use super::*;

    fn one<E: Target>() -> (UnitDb<E>, UnitId, PublicId, Vec<u8>, Vec<u8>) {
        let disk = SimVfs::new(14);
        let db = open::<E>(&disk);
        let (unit, token) = issue(&db, b"t");
        let store = raw(&db);
        let tk = store.iter().find(|(k, _)| k[0] == 0x70).unwrap().0.clone();
        let uk = store.iter().find(|(k, _)| k[0] == 0x71).unwrap().0.clone();
        (db, unit, token, tk, uk)
    }

    fn put<E: Target>(db: &UnitDb<E>, k: &[u8], v: Vec<u8>) {
        db.raw_commit(vec![(k.to_vec(), Some(v))]).unwrap();
    }

    fn put_records<E: Target>(db: &UnitDb<E>, unit: UnitId, token: &PublicId) {
        let entries = records(unit, token).into_iter().map(|(k, v)| (k, Some(v))).collect();
        db.raw_commit(entries).unwrap();
    }

    fn other_token(n: u8) -> PublicId {
        PublicId::from_bytes(std::array::from_fn(|i| n.wrapping_mul(17).wrapping_add(i as u8)))
    }

    fn rules<E: Target>(db: &UnitDb<E>) -> Vec<String> {
        db.verify_units().unwrap().findings.iter().map(|f| f.rule.id()).collect()
    }

    fn is_corruption(e: &UnitError) -> bool {
        matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_)))
    }

    pub fn disagreeing_records_are_corruption<E: Target>() {
        let (db, unit, token, _, uk) = one::<E>();
        let mut other = vec![1u8, 0];
        other.extend_from_slice(&[7u8; 16]);
        put(&db, &uk, other);
        let e = db.resolve_public_id(&token).unwrap_err();
        assert!(is_corruption(&e), "{e:?}");
        // The unit's own token is not served either: it does not resolve.
        let e = db.public_id(unit).unwrap_err();
        assert!(is_corruption(&e), "{e:?}");
    }

    pub fn a_token_record_without_its_reverse_is_corruption<E: Target>() {
        let (db, unit, token, _, uk) = one::<E>();
        db.raw_commit(vec![(uk, None)]).unwrap();
        let e = db.resolve_public_id(&token).unwrap_err();
        assert!(is_corruption(&e), "{e:?}");
        assert_eq!(db.public_id(unit).unwrap(), None);
    }

    /// D-c: `public_id` never returns a token that does not resolve.
    pub fn a_reverse_record_without_its_token_is_corruption_in_public_id<E: Target>() {
        let (db, unit, token, tk, _) = one::<E>();
        db.raw_commit(vec![(tk.clone(), None)]).unwrap();
        let e = db.public_id(unit).unwrap_err();
        assert!(is_corruption(&e), "{e:?}");
        assert_eq!(db.resolve_public_id(&token).unwrap(), None);
        // A token record that names another unit is refused the same way.
        let (db, unit, _, tk, _) = one::<E>();
        let mut elsewhere = vec![1u8, 0];
        elsewhere.extend_from_slice(&[9u8; 16]);
        put(&db, &tk, elsewhere);
        let e = db.public_id(unit).unwrap_err();
        assert!(is_corruption(&e), "{e:?}");
    }

    /// D-d: no error text pairs a token with the internal unit id.
    pub fn error_texts_never_pair_a_token_with_a_unit_id<E: Target>() {
        let (db, unit, token, tk, uk) = one::<E>();
        let (token_text, unit_text) = (token.to_string(), unit.to_string());
        let pair = |text: &str| text.contains(&token_text) && text.contains(&unit_text);
        let mut texts = Vec::new();
        // Each way a pair can disagree, then read from both ends.
        let good_uk = db.raw_get(&uk).unwrap().unwrap();
        let good_tk = db.raw_get(&tk).unwrap().unwrap();
        let mut other = vec![1u8, 0];
        other.extend_from_slice(&[7u8; 16]);
        put(&db, &uk, other.clone());
        texts.extend([db.resolve_public_id(&token).unwrap_err(), db.public_id(unit).unwrap_err()]);
        put(&db, &uk, good_uk.clone());
        db.raw_commit(vec![(uk.clone(), None)]).unwrap();
        texts.push(db.resolve_public_id(&token).unwrap_err());
        put(&db, &uk, good_uk);
        db.raw_commit(vec![(tk.clone(), None)]).unwrap();
        texts.push(db.public_id(unit).unwrap_err());
        put(&db, &tk, good_tk);
        // A damaged record, and the findings of the verifier on all of them.
        put(&db, &tk, vec![9, 0]);
        texts.push(db.resolve_public_id(&token).unwrap_err());
        for e in texts {
            assert!(!pair(&e.to_string()), "{e}");
        }
        for f in db.verify_units().unwrap().findings {
            assert!(!pair(&f.detail), "{}", f.detail);
            assert!(!f.detail.contains(&token_text) && !f.detail.contains(&unit_text), "{}", f.detail);
        }
    }

    pub fn verifier_reports_each_malformed_record<E: Target>() {
        let (db, _, _, tk, uk) = one::<E>();
        let tweak = |k: &[u8], f: &dyn Fn(&mut Vec<u8>)| {
            let mut k = k.to_vec();
            f(&mut k);
            k
        };
        let v = db.raw_get(&tk).unwrap().unwrap();
        assert_eq!(v.len(), 18);
        // (key, value, the one rule the verifier reports for the pair)
        let cases: Vec<(Vec<u8>, Vec<u8>, UnitRule)> = vec![
            // A 0x70 key one byte short and one byte long; a 0x71 key likewise.
            (
                tweak(&tk, &|k| {
                    k.pop();
                }),
                v.clone(),
                UnitRule::Unit3,
            ),
            (tweak(&tk, &|k| k.push(0)), v.clone(), UnitRule::Unit3),
            (
                tweak(&uk, &|k| {
                    k.pop();
                }),
                v.clone(),
                UnitRule::Unit3,
            ),
            (tweak(&uk, &|k| k.push(0)), v.clone(), UnitRule::Unit3),
            (tweak(&tk, &|k| k.truncate(1)), v.clone(), UnitRule::Unit3),
            // The value: length, then each field of the envelope.
            (tk.clone(), v[..17].to_vec(), UnitRule::Unit9),
            (tk.clone(), v[..2].to_vec(), UnitRule::Unit9),
            (tk.clone(), v[..1].to_vec(), UnitRule::Unit9),
            (tk.clone(), Vec::new(), UnitRule::Unit9),
            (tk.clone(), [v.clone(), vec![0]].concat(), UnitRule::Unit10),
            (uk.clone(), tweak(&v, &|b| b[0] = 0), UnitRule::Unit6),
            (uk.clone(), tweak(&v, &|b| b[0] = 2), UnitRule::Unit7),
            (uk.clone(), tweak(&v, &|b| b[1] = 1), UnitRule::Unit8),
            // The envelope is read before the key's length and before the
            // value's length: a newer shape is a newer layer, not damage.
            (
                tweak(&tk, &|k| {
                    k.pop();
                }),
                tweak(&v, &|b| b[0] = 2),
                UnitRule::Unit7,
            ),
            (tk.clone(), [tweak(&v, &|b| b[0] = 2), vec![0; 4]].concat(), UnitRule::Unit7),
        ];
        let (good_tk, good_uk) = (db.raw_get(&tk).unwrap().unwrap(), db.raw_get(&uk).unwrap().unwrap());
        for (i, (k, bad, rule)) in cases.into_iter().enumerate() {
            put(&db, &k, bad);
            let report = db.verify_units().unwrap();
            let got: Vec<_> = report.findings.iter().map(|f| f.rule).collect();
            assert_eq!(got, vec![VerifyRule::Structure(rule)], "case {i}, {rule}: {:?}", report.findings);
            put(&db, &tk, good_tk.clone());
            put(&db, &uk, good_uk.clone());
            if k != tk && k != uk {
                db.raw_commit(vec![(k, None)]).unwrap();
            }
            clean(&db);
        }
        // A 0x70 record of a newer envelope is a newer layer, not damage.
        put(&db, &tk, tweak(&v, &|b| b[0] = 2));
        let e = db.verify_units().unwrap().into_result().unwrap_err();
        assert!(!is_corruption(&e), "{e:?}");
        put(&db, &tk, good_tk);
        clean(&db);
    }

    /// UNIT-39: a token record without the reverse record of its unit.
    pub fn cross_a_token_without_its_reverse<E: Target>() {
        let (db, _, _, _, uk) = one::<E>();
        db.raw_commit(vec![(uk, None)]).unwrap();
        assert_eq!(rules(&db), vec!["UNIT-39"]);
    }

    /// UNIT-40: a reverse record without the token record it names.
    pub fn cross_a_reverse_without_its_token<E: Target>() {
        let (db, _, _, tk, _) = one::<E>();
        db.raw_commit(vec![(tk, None)]).unwrap();
        assert_eq!(rules(&db), vec!["UNIT-40"]);
    }

    /// UNIT-41: a pair that names a unit that does not exist.
    pub fn cross_a_pair_naming_a_unit_that_does_not_exist<E: Target>() {
        let (db, _, _, _, _) = one::<E>();
        put_records(&db, UnitId([0xee; 16]), &other_token(1));
        assert_eq!(rules(&db), vec!["UNIT-41"]);
        // Only the reverse record exists: the missing token is UNIT-40, and the
        // missing unit UNIT-41 on the reverse.
        let (db, _, _, _, _) = one::<E>();
        let [_, (rk, rv)] = records(UnitId([0xee; 16]), &other_token(2));
        put(&db, &rk, rv);
        let mut got = rules(&db);
        got.sort();
        assert_eq!(got, vec!["UNIT-40", "UNIT-41"]);
    }

    /// A pair left on a deleted unit (damage, or an older writer) is its own
    /// UNIT-41 finding, resolves like a token never issued, and is not exported.
    pub fn cross_a_pair_left_on_a_deleted_unit<E: Target>() {
        let (db, unit, token, _, _) = one::<E>();
        delete(&db, unit);
        clean(&db);
        put_records(&db, unit, &token);
        let found = rules(&db);
        assert!(found.contains(&"UNIT-41".to_string()), "{found:?}");
        assert_eq!(db.resolve_public_id(&token).unwrap(), None);
        assert_eq!(db.public_id(unit).unwrap(), None);
        let target = open_from::<E>(&SimVfs::new(30), 100);
        let report = import(&target, &export(&db).unwrap()).unwrap();
        assert_eq!(report.tokens, 0);
        assert_eq!(target.resolve_public_id(&token).unwrap(), None);
    }

    /// UNIT-42: two tokens for one unit.
    pub fn cross_two_tokens_for_one_unit<E: Target>() {
        let (db, unit, _, _, _) = one::<E>();
        let [(second_key, second_value), _] = records(unit, &other_token(3));
        put(&db, &second_key, second_value);
        assert_eq!(rules(&db), vec!["UNIT-42"]);
        // The reverse record names the second token instead: the first is the extra one.
        let (db, unit, _, _, _) = one::<E>();
        let [_, (reverse_key, reverse_value)] = records(unit, &other_token(4));
        put(&db, &reverse_key, reverse_value);
        let mut got = rules(&db);
        got.sort();
        assert_eq!(got, vec!["UNIT-40", "UNIT-42"]);
    }

    /// UNIT-43: one token for two units.
    pub fn cross_one_token_for_two_units<E: Target>() {
        let (db, _, token, _, _) = one::<E>();
        let second = plain(&db, b"second");
        let [_, (reverse_key, reverse_value)] = records(second, &token);
        put(&db, &reverse_key, reverse_value);
        assert_eq!(rules(&db), vec!["UNIT-43"]);
    }

    /// UNIT-44: token records in a database whose mark is below layer 3.
    pub fn cross_tokens_under_a_mark_below_layer_3<E: Target>() {
        let (db, _, _, _, _) = one::<E>();
        let mark = LayerMeta::decode_upto(&mark_of(&db), MAX_LAYER_VERSION).unwrap();
        for lowered in [1, 2] {
            let v = LayerMeta { layer_version: lowered, ..mark.clone() }.encode().unwrap();
            put(&db, &mark_key(), v);
            assert_eq!(rules(&db), vec!["UNIT-44"], "mark {lowered}");
        }
        put(&db, &mark_key(), mark.encode().unwrap());
        clean(&db);
    }

    /// A record that does not decode is one finding: its pair is not
    /// reported again as missing.
    pub fn cross_a_damaged_record_is_not_reported_twice<E: Target>() {
        let (db, _, _, _, uk) = one::<E>();
        put(&db, &uk, vec![1, 0, 1]);
        assert_eq!(rules(&db), vec!["UNIT-9"]);
        let (db, _, _, tk2, _) = one::<E>();
        put(&db, &tk2, vec![]);
        assert_eq!(rules(&db), vec!["UNIT-9"]);
    }

    pub fn cross_counts_the_records<E: Target>() {
        let (db, _, _, _, _) = one::<E>();
        assert_eq!(db.verify_units().unwrap().counts.public_id_records, 2);
        let [(k, v), _] = records(UnitId([0xee; 16]), &other_token(5));
        put(&db, &k, v);
        assert_eq!(db.verify_units().unwrap().counts.public_id_records, 3);
    }
}

macro_rules! on_engine {
    ($e:ty; $($name:ident),* $(,)?) => {$(
        #[test]
        fn $name() {
            super::$name::<$e>();
        }
    )*};
}

#[cfg(feature = "test-support")]
macro_rules! on_engine_stored {
    ($e:ty; $($name:ident),* $(,)?) => {$(
        #[test]
        fn $name() {
            super::super::stored::$name::<$e>();
        }
    )*};
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            on_engine!($e;
                issued_token_reads_back_and_resolves,
                stable_after_reopen_and_later_revisions,
                mark_is_raised_to_3_by_the_first_token_and_never_lowered,
                layer2_reader_refuses_a_database_with_tokens,
                deleting_a_unit_removes_its_token_like_one_never_issued,
                retention_leaves_the_mapping_of_a_live_unit,
                same_transaction_a_dropped_change_set_leaves_nothing,
                same_transaction_a_failed_sync_leaves_neither_unit_nor_token,
                same_transaction_a_synced_commit_survives_power_loss_whole,
                tampered_but_well_formed_tokens_name_nothing,
                opaque_tokens_follow_neither_id_nor_creation_order,
                legacy_units_without_a_token_stay_readable,
                record_layout_is_pinned,
                record_verifier_accepts_tokens,
                export_carries_tokens_and_import_restores_them,
                export_import_export_keeps_every_token_across_two_hops,
                import_refuses_one_token_for_two_units,
                import_refuses_token_records_that_no_writer_produces,
                import_of_a_version_0_export_is_refused,
                derived_db_issues_resolves_and_forgets_tokens,
                fixture_directory_opens_resolves_and_verifies,
                fixture_directory_is_refused_by_a_layer2_reader,
            );
            #[cfg(feature = "test-support")]
            mod stored {
                on_engine_stored!($e;
                    disagreeing_records_are_corruption,
                    a_token_record_without_its_reverse_is_corruption,
                    a_reverse_record_without_its_token_is_corruption_in_public_id,
                    error_texts_never_pair_a_token_with_a_unit_id,
                    verifier_reports_each_malformed_record,
                    cross_a_token_without_its_reverse,
                    cross_a_reverse_without_its_token,
                    cross_a_pair_naming_a_unit_that_does_not_exist,
                    cross_a_pair_left_on_a_deleted_unit,
                    cross_two_tokens_for_one_unit,
                    cross_one_token_for_two_units,
                    cross_tokens_under_a_mark_below_layer_3,
                    cross_a_damaged_record_is_not_reported_twice,
                    cross_counts_the_records,
                );
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
