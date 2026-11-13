//! E9-02: public identifiers (ADR-0005 option 1, invariant I-27).
//!
//! * `issued_*`, `stable_*`: a token is issued with its unit, reads back, and
//!   is the same after a clean reopen and after later revisions.
//! * `same_transaction_*`: the token records commit with the unit or not at
//!   all (a dropped change set and a failed sync leave neither).
//! * `tampered_*`: every way a text can be wrong is refused by the parser
//!   with its own error, and a well-formed token nobody holds resolves to
//!   nothing.
//! * `opaque_*`: the token carries nothing of the unit id or the creation
//!   order, and resolving it grants nothing.
//! * `legacy_*`: units without a token stay readable, and a directory
//!   written before this change still opens (committed fixture).
//! * `record_*`: the layout of the two stored records, and the verifier.
//!
//! Every database test runs on `emulated_log` and on `kerno_db`.

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use harness::units::{Target, open_units, raw};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::vfs::{CrashKind, DirImage, OpKind, SimVfs};
use kerno_units::export::export;
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

fn clean<E: Target>(db: &UnitDb<E>) {
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
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

fn a_deleted_unit_still_resolves_but_shows_no_content<E: Target>() {
    let disk = SimVfs::new(3);
    let db = open::<E>(&disk);
    let (unit, token) = issue(&db, b"secret");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(unit, None).unwrap();
    cs.commit().unwrap();
    // The token only names the unit (I-27); what it is in is the layer's to say.
    let named = db.resolve_public_id(&token).unwrap().expect("still named");
    let u = db.get_unit(named).unwrap().unwrap();
    assert_eq!((u.lifecycle, u.payload), (Lifecycle::Deleted, None));
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

fn same_transaction_a_failed_sync_leaves_neither_unit_nor_token<E: Target>() {
    let disk = SimVfs::new(5);
    let db = open::<E>(&disk);
    let (kept, kept_token) = issue(&db, b"kept");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let (unit, token) = cs.create_public(new_unit(b"lost")).unwrap();
    disk.fail_when(|op| op.kind == OpKind::SyncData, 1, None);
    let result = cs.commit();
    assert!(disk.fault_injected(), "the fault did not fire");
    assert!(result.is_err());
    let image = disk.crash_now(CrashKind::PowerLoss);
    let after = open::<E>(&image);
    assert_eq!(after.resolve_public_id(&kept_token).unwrap(), Some(kept));
    // The commit is all there or not at all (I-21): both halves, or neither.
    let unit_there = after.get_unit(unit).unwrap().is_some();
    assert_eq!(after.public_id(unit).unwrap().is_some(), unit_there);
    assert_eq!(after.resolve_public_id(&token).unwrap().is_some(), unit_there);
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
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let old = cs.create(new_unit(b"old")).unwrap();
    cs.commit().unwrap();
    // An import has no tokens either (the draft export does not carry them).
    let src = open::<E>(&SimVfs::new(10));
    let (imported_src, _) = issue(&src, b"carried");
    let target_disk = SimVfs::new(11);
    let target = open::<E>(&target_disk);
    import(&target, &export(&src).unwrap()).unwrap();
    for (db, unit, payload) in [(&db, old, &b"old"[..]), (&target, imported_src, &b"carried"[..])] {
        assert_eq!(db.public_id(unit).unwrap(), None);
        assert_eq!(db.get_unit(unit).unwrap().unwrap().payload.as_deref(), Some(payload));
        clean(db);
    }
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
/// unit and no token.
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
    clean(&db);
    // New units on top of it get tokens; the old one still has none.
    let (fresh, token) = issue(&db, b"after");
    assert_eq!(db.resolve_public_id(&token).unwrap(), Some(fresh));
    assert_eq!(db.public_id(unit).unwrap(), None);
    clean(&db);
}

// ----------------------------------------------------------------- records

/// The stored layout, byte for byte: tag, 16 bytes, then `01` and 16 bytes.
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
    assert_eq!((tk.len(), tv.len(), uk.len(), uv.len()), (17, 17, 17, 17));
    assert_eq!(tv[0], 1);
    assert_eq!(uv[0], 1);
    assert_eq!(&tv[1..], &unit.0);
    assert_eq!(&uk[1..], &unit.0);
    assert_eq!(&tk[1..], &uv[1..], "the two records name the same token");
    // The key bytes are the 128 bits the text encodes (RFC 4648, lower case).
    assert_eq!(text, format!("k1_{}", base32(&tk[1..])));
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
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
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

    pub fn disagreeing_records_are_corruption<E: Target>() {
        let (db, _, token, _, uk) = one::<E>();
        let mut other = vec![1u8];
        other.extend_from_slice(&[7u8; 16]);
        put(&db, &uk, other);
        let e = db.resolve_public_id(&token).unwrap_err();
        assert!(matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_))), "{e:?}");
    }

    pub fn a_token_record_without_its_reverse_is_corruption<E: Target>() {
        let (db, _, token, _, uk) = one::<E>();
        db.raw_commit(vec![(uk, None)]).unwrap();
        let e = db.resolve_public_id(&token).unwrap_err();
        assert!(matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_))), "{e:?}");
    }

    pub fn verifier_reports_each_malformed_record<E: Target>() {
        let (db, _, _, tk, uk) = one::<E>();
        let mut short_key = tk.clone();
        short_key.pop();
        let mut v = vec![1u8];
        v.extend_from_slice(&[0u8; 16]);
        let cases: Vec<(Vec<u8>, Vec<u8>, UnitRule)> = vec![
            (short_key, v.clone(), UnitRule::Unit3),
            (tk.clone(), v[..16].to_vec(), UnitRule::Unit9),
            (tk.clone(), [v.clone(), vec![0]].concat(), UnitRule::Unit10),
            (uk.clone(), [vec![0u8], v[1..].to_vec()].concat(), UnitRule::Unit6),
            (uk.clone(), [vec![2u8], v[1..].to_vec()].concat(), UnitRule::Unit7),
            (uk.clone(), Vec::new(), UnitRule::Unit9),
        ];
        let (good_tk, good_uk) = (db.raw_get(&tk).unwrap().unwrap(), db.raw_get(&uk).unwrap().unwrap());
        for (k, bad, rule) in cases {
            put(&db, &k, bad);
            let report = db.verify_units().unwrap();
            let rules: Vec<_> = report.findings.iter().map(|f| f.rule).collect();
            assert_eq!(rules, vec![VerifyRule::Structure(rule)], "{rule}: {:?}", report.findings);
            put(&db, &tk, good_tk.clone());
            put(&db, &uk, good_uk.clone());
            if k.len() == 16 {
                db.raw_commit(vec![(k, None)]).unwrap();
            }
            clean(&db);
        }
    }
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            #[test]
            fn issued_token_reads_back_and_resolves() {
                super::issued_token_reads_back_and_resolves::<$e>();
            }
            #[test]
            fn stable_after_reopen_and_later_revisions() {
                super::stable_after_reopen_and_later_revisions::<$e>();
            }
            #[test]
            fn a_deleted_unit_still_resolves_but_shows_no_content() {
                super::a_deleted_unit_still_resolves_but_shows_no_content::<$e>();
            }
            #[test]
            fn same_transaction_a_dropped_change_set_leaves_nothing() {
                super::same_transaction_a_dropped_change_set_leaves_nothing::<$e>();
            }
            #[test]
            fn same_transaction_a_failed_sync_leaves_neither_unit_nor_token() {
                super::same_transaction_a_failed_sync_leaves_neither_unit_nor_token::<$e>();
            }
            #[test]
            fn tampered_but_well_formed_tokens_name_nothing() {
                super::tampered_but_well_formed_tokens_name_nothing::<$e>();
            }
            #[test]
            fn opaque_tokens_follow_neither_id_nor_creation_order() {
                super::opaque_tokens_follow_neither_id_nor_creation_order::<$e>();
            }
            #[test]
            fn legacy_units_without_a_token_stay_readable() {
                super::legacy_units_without_a_token_stay_readable::<$e>();
            }
            #[test]
            fn record_layout_is_pinned() {
                super::record_layout_is_pinned::<$e>();
            }
            #[test]
            fn record_verifier_accepts_tokens() {
                super::record_verifier_accepts_tokens::<$e>();
            }
            #[cfg(feature = "test-support")]
            #[test]
            fn stored_disagreeing_records_are_corruption() {
                super::stored::disagreeing_records_are_corruption::<$e>();
            }
            #[cfg(feature = "test-support")]
            #[test]
            fn stored_token_record_without_its_reverse_is_corruption() {
                super::stored::a_token_record_without_its_reverse_is_corruption::<$e>();
            }
            #[cfg(feature = "test-support")]
            #[test]
            fn stored_verifier_reports_each_malformed_record() {
                super::stored::verifier_reports_each_malformed_record::<$e>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
