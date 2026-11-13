//! E9-05: import of the draft export into a new database (I-30, D10), over
//! `Emulated<LogEngine>` and `KernoDb`.
//!
//! * `roundtrip_*` — export, import into a new directory, export again: the
//!   same bytes, the same public reads, a clean verifier, and the new database
//!   takes further writes. Across engines too.
//! * `format_errors_*` — each malformed file is refused with the class of
//!   `UnitError` the design names, and a refused import writes nothing.
//! * `target_*` — only an empty database with the exported default policy
//!   takes an import.

use std::path::Path;
use std::sync::Arc;

use kerno_core::Error as CoreError;
use kerno_core::contract::{Options, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::export::{DIGEST_LEN, export};
use kerno_units::import::{ImportReport, MAX_REPLAY_SEQ, import};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg(default: HistoryPolicy) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c.default_policy = default;
    c
}

fn open_with<E: TxnEngine>(default: HistoryPolicy) -> UnitDb<E> {
    UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &cfg(default)).expect("open")
}

fn open<E: TxnEngine>() -> UnitDb<E> {
    open_with(HistoryPolicy::Versioned)
}

fn new_unit(t: &str, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: 1, metadata: b"m".to_vec(), payload: payload.to_vec() }
}

fn upd(payload: &[u8]) -> UnitUpdate {
    UnitUpdate { schema_version: None, metadata: None, payload: payload.to_vec() }
}

fn by(actor: &[u8], source: &[u8]) -> ChangeMeta {
    ChangeMeta { actor: Some(actor.to_vec()), source: Some(source.to_vec()), ..ChangeMeta::default() }
}

/// A history with every revision kind, a gap in the sequence, policies, and
/// one relation of each provenance.
fn populate<E: TxnEngine>(db: &UnitDb<E>) -> Vec<UnitId> {
    db.set_policy("log", HistoryPolicy::None, ChangeMeta::default()).unwrap();
    let mut cs = db.begin_changeset(by(b"ann", b"cli")).unwrap();
    let a = cs.create(new_unit("doc", b"a-1")).unwrap();
    let b = cs.create(new_unit("doc", b"b-1")).unwrap();
    let c = cs.create(new_unit("log", b"c-1")).unwrap();
    let d = cs.create(new_unit("doc", b"")).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(by(b"bob", b"api")).unwrap();
    cs.update(a, upd(b"a-2"), None).unwrap();
    cs.update(c, upd(b"c-2"), None).unwrap();
    cs.archive(b, None).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.unarchive(b, None).unwrap();
    cs.restore(a, RevNo(1), None).unwrap();
    cs.update(c, upd(b"c-3"), None).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(a, b, "cites", RelationProvenance::Declared).unwrap();
    cs.create_relation(b, c, "cites", RelationProvenance::Imported { source: b"wiki".to_vec() }).unwrap();
    cs.create_relation(c, a, "likes", RelationProvenance::Algorithmic { origin: b"walk".to_vec() }).unwrap();
    cs.create_relation(
        a,
        a,
        "self",
        RelationProvenance::Inferred { origin: b"guess".to_vec(), confidence_permille: 640 },
    )
    .unwrap();
    cs.commit().unwrap();
    // A relation-only change set, then a delete.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(a, c, "cites", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta { actor: Some(b"eve".to_vec()), ..ChangeMeta::default() }).unwrap();
    cs.delete(d, None).unwrap();
    cs.commit().unwrap();
    db.set_policy("doc", HistoryPolicy::Versioned, ChangeMeta::default()).unwrap();
    vec![a, b, c, d]
}

/// A unit under policy `None` answers `HistoryUnavailable`: both must.
fn history<E: TxnEngine>(db: &UnitDb<E>, id: UnitId) -> Result<Vec<Revision>, String> {
    let q = HistoryQuery { with_payload: true, ..HistoryQuery::oldest(100) };
    db.history(id, q).map_err(|e| format!("{e:?}"))
}

fn relations<E: TxnEngine>(db: &UnitDb<E>, id: UnitId) -> Vec<Relation> {
    let snap = db.snapshot().unwrap();
    let mut all = snap.relations_of(id, Direction::Out, None).unwrap();
    all.extend(snap.relations_of(id, Direction::In, None).unwrap());
    all
}

/// Same public reads on both databases.
fn same_state<A: TxnEngine, B: TxnEngine>(a: &UnitDb<A>, b: &UnitDb<B>, ids: &[UnitId]) {
    assert_eq!(export(a).unwrap(), export(b).unwrap());
    for &id in ids {
        assert_eq!(a.get_unit(id).unwrap(), b.get_unit(id).unwrap());
        assert_eq!(history(a, id), history(b, id));
        assert_eq!(relations(a, id), relations(b, id));
    }
    // The change sets rebuilt from the revisions keep who and what.
    for seq in [2, 3, 4, 7] {
        let id = CommitSeq(seq);
        let (x, y) = (a.change_set(id).unwrap().unwrap(), b.change_set(id).unwrap().unwrap());
        // The draft keeps no revision below a unit's oldest one, so the
        // change set lists no member the source had already pruned.
        // The ordinals follow the unit id, not the order of the operations.
        let mut kept: Vec<_> = x
            .members
            .into_iter()
            .filter(|m| a.get_unit(m.revision.unit).unwrap().unwrap().oldest <= m.revision.rev)
            .collect();
        let mut got = y.members;
        kept.sort_by_key(|m| m.revision);
        got.sort_by_key(|m| m.revision);
        assert_eq!((x.purpose, x.provenance, kept), (y.purpose, y.provenance, got), "change set {seq}");
    }
    for t in ["doc", "log"] {
        assert_eq!(a.policy(t).unwrap(), b.policy(t).unwrap());
    }
}

fn clean<E: TxnEngine>(db: &UnitDb<E>) {
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
}

fn roundtrip<A: TxnEngine, B: TxnEngine>() {
    let src = open::<A>();
    let ids = populate(&src);
    let bytes = export(&src).unwrap();

    let dst = open::<B>();
    let report = import(&dst, &bytes).unwrap();
    assert_eq!(report.units, 4);
    assert_eq!(report.relations, 5);
    assert_eq!(report.policies, 2);
    assert!(report.revisions >= 4 + 3, "{report:?}");
    clean(&dst);
    same_state(&src, &dst, &ids);

    // The new database is a database: it takes a write after the imported
    // sequence, and the old one does not see it.
    let seq = dst.snapshot().unwrap().seq();
    let mut cs = dst.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(ids[0], upd(b"after"), None).unwrap();
    let info = cs.commit().unwrap();
    assert!(info.commit_seq > seq);
    clean(&dst);
    assert_ne!(export(&src).unwrap(), export(&dst).unwrap());
    assert_eq!(src.get_unit(ids[0]).unwrap().unwrap().payload.as_deref(), Some(&b"a-1"[..]));
    assert_eq!(dst.get_unit(ids[0]).unwrap().unwrap().payload.as_deref(), Some(&b"after"[..]));
}

#[test]
fn roundtrip_emulated_log() {
    roundtrip::<EmuLog, EmuLog>();
}

#[test]
fn roundtrip_kerno_db() {
    roundtrip::<KernoDb, KernoDb>();
}

#[test]
fn roundtrip_across_engines() {
    roundtrip::<EmuLog, KernoDb>();
    roundtrip::<KernoDb, EmuLog>();
}

fn empty_database<E: TxnEngine>() {
    // No mark, nothing in it: the export imports to the same nothing.
    let src = open::<E>();
    let bytes = export(&src).unwrap();
    let dst = open::<E>();
    assert_eq!(import(&dst, &bytes).unwrap(), ImportReport::default());
    assert_eq!(export(&dst).unwrap(), bytes);
    // A mark and a policy, no unit.
    let src = open::<E>();
    src.set_policy("x", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    let bytes = export(&src).unwrap();
    let dst = open::<E>();
    assert_eq!(import(&dst, &bytes).unwrap().policies, 1);
    assert_eq!(export(&dst).unwrap(), bytes);
    clean(&dst);
}

#[test]
fn empty_database_emulated_log() {
    empty_database::<EmuLog>();
}

#[test]
fn empty_database_kerno_db() {
    empty_database::<KernoDb>();
}

/// A history under `Audit` keeps one revision's content only: the draft
/// leaves the others out, so it cannot be imported, and says why.
fn trimmed_history<E: TxnEngine>() {
    let src = open::<E>();
    src.set_policy("note", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    let (id, _) = src.create_unit(new_unit("note", b"one"), ChangeMeta::default()).unwrap();
    let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, upd(b"two"), None).unwrap();
    cs.commit().unwrap();
    // One revision of an Audit unit is importable.
    let single = open::<E>();
    single.set_policy("note", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    single.create_unit(new_unit("note", b"one"), ChangeMeta::default()).unwrap();
    let dst = open::<E>();
    import(&dst, &export(&single).unwrap()).unwrap();
    assert_eq!(export(&dst).unwrap(), export(&single).unwrap());
    // Two are not.
    let dst = open::<E>();
    let err = import(&dst, &export(&src).unwrap()).unwrap_err();
    assert!(matches!(err, UnitError::Storage(CoreError::Corruption(_))), "{err:?}");
    assert!(err.to_string().contains("cannot be imported"), "{err}");
}

#[test]
fn trimmed_history_emulated_log() {
    trimmed_history::<EmuLog>();
}

#[test]
fn trimmed_history_kerno_db() {
    trimmed_history::<KernoDb>();
}

/// Closes a body with the digest the format asks for.
fn seal(mut body: Vec<u8>) -> Vec<u8> {
    let digest = blake3::hash(&body);
    body.extend_from_slice(digest.as_bytes());
    body
}

fn body(bytes: &[u8]) -> Vec<u8> {
    bytes[..bytes.len() - DIGEST_LEN].to_vec()
}

/// A file a payload rule refuses while the file is read: the new target is
/// left without a commit (the verifier that closes an import gives the same
/// rule, but only after writing).
fn rule<E: TxnEngine>(bytes: &[u8]) -> String {
    let dst = open::<E>();
    let m = match import(&dst, bytes) {
        Err(UnitError::Storage(CoreError::Corruption(m))) if m.starts_with("UNIT-") => m,
        other => panic!("expected a UNIT rule, got {other:?}"),
    };
    assert_eq!(dst.snapshot().unwrap().seq(), CommitSeq(0), "{m}: refused after a write");
    m
}

fn corrupt(r: Result<ImportReport, UnitError>) -> String {
    match r {
        // Refused by the importer's own checks, not by the verifier after it.
        Err(UnitError::Storage(CoreError::Corruption(m))) if m.starts_with("export: ") => m,
        other => panic!("expected a corruption error, got {other:?}"),
    }
}

fn format_errors<E: TxnEngine>() {
    let src = open::<E>();
    populate(&src);
    let good = export(&src).unwrap();
    let fresh = || open::<E>();

    // Short, truncated at every length from the header on, wrong magic.
    corrupt(import(&fresh(), &good[..10]));
    for cut in [16 + DIGEST_LEN, good.len() / 2, good.len() - 1] {
        corrupt(import(&fresh(), &good[..cut]));
    }
    let mut bad = good.clone();
    bad[0] = b'X';
    assert!(corrupt(import(&fresh(), &bad)).contains("magic"));
    // The reserved magic is not this format.
    let mut bad = body(&good);
    bad[..8].copy_from_slice(b"KRNO_EXP");
    corrupt(import(&fresh(), &seal(bad)));
    // A flipped byte anywhere breaks the digest.
    let mut bad = good.clone();
    bad[40] ^= 1;
    assert!(corrupt(import(&fresh(), &bad)).contains("digest"));
    let mut bad = good.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert!(corrupt(import(&fresh(), &bad)).contains("digest"));
    // Flags and reserved bytes.
    for (at, v) in [(12, 0u8), (12, 3), (13, 1)] {
        let mut bad = body(&good);
        bad[at] = v;
        corrupt(import(&fresh(), &seal(bad)));
    }
    // Another format version is an incompatibility, not a corruption.
    let mut other = body(&good);
    other[8..12].copy_from_slice(&2u32.to_be_bytes());
    let err = import(&fresh(), &seal(other)).unwrap_err();
    assert!(matches!(err, UnitError::Storage(CoreError::Incompatible(_))), "{err:?}");
    // Bytes after the last section.
    let mut extra = body(&good);
    extra.push(0);
    assert!(corrupt(import(&fresh(), &seal(extra))).contains("after the last section"));
    // A default policy byte outside the table.
    let mut bad = body(&good);
    bad[16] = 9;
    corrupt(import(&fresh(), &seal(bad)));
    // Content without a layer mark.
    let mut bad = body(&good);
    bad[16] = 0xFF;
    assert!(corrupt(import(&fresh(), &seal(bad))).contains("layer mark"));
    // A mark with nothing under it.
    let mut empty = body(&export(&open::<E>()).unwrap());
    empty[16] = 2;
    assert!(corrupt(import(&fresh(), &seal(empty))).contains("layer mark"));

    // A refused file left the target as it was: still importable.
    let dst = fresh();
    corrupt(import(&dst, &good[..good.len() - 1]));
    assert_eq!(dst.snapshot().unwrap().seq(), CommitSeq(0));
    import(&dst, &good).unwrap();
    assert_eq!(export(&dst).unwrap(), good);
}

#[test]
fn format_errors_emulated_log() {
    format_errors::<EmuLog>();
}

#[test]
fn format_errors_kerno_db() {
    format_errors::<KernoDb>();
}

/// Layout of the export of one `doc` unit with one revision and no policy:
/// header 16, default 1, policy count 4, token count 4, unit count 4, id 16, type 4 + 3.
const CREATED_SEQ_AT: usize = 16 + 1 + 4 + 4 + 4 + 16 + 4 + 3;
/// After `created_seq`, `created_at`, `oldest`, `head`, `r`, `rev`, `kind`
/// and an absent `parent`.
const COMMIT_SEQ_AT: usize = CREATED_SEQ_AT + 8 + 8 + 8 + 8 + 4 + 8 + 1 + 1;

fn structure_errors<E: TxnEngine>() {
    let src = open::<E>();
    src.create_unit(new_unit("doc", b"x"), ChangeMeta::default()).unwrap();
    let good = body(&export(&src).unwrap());
    assert_eq!(good[CREATED_SEQ_AT..CREATED_SEQ_AT + 8], 1u64.to_be_bytes());
    assert_eq!(good[COMMIT_SEQ_AT..COMMIT_SEQ_AT + 8], 1u64.to_be_bytes());
    import(&open::<E>(), &seal(good.clone())).unwrap();

    let patched = |at: usize, v: &[u8]| {
        let mut b = good.clone();
        b[at..at + v.len()].copy_from_slice(v);
        seal(b)
    };
    // The unit and its revision disagree on the creating sequence.
    corrupt(import(&open::<E>(), &patched(CREATED_SEQ_AT, &2u64.to_be_bytes())));
    // A payload that is not the one hashed: its last byte sits before the
    // closing relation count.
    let mut bad = good.clone();
    let payload_at = bad.len() - 4 - 1;
    assert_eq!(bad[payload_at], b'x');
    bad[payload_at] ^= 1;
    assert!(rule::<E>(&seal(bad)).starts_with("UNIT-28"));
    // The head names more revisions than the file holds.
    corrupt(import(&open::<E>(), &patched(CREATED_SEQ_AT + 24, &2u64.to_be_bytes())));
    // A kind outside the table.
    corrupt(import(&open::<E>(), &patched(COMMIT_SEQ_AT - 2, &[9])));
    // Two revisions of one unit with the same commit sequence.
    let two = open::<E>();
    let (id, _) = two.create_unit(new_unit("doc", b"x"), ChangeMeta::default()).unwrap();
    let mut cs = two.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, upd(b"y"), None).unwrap();
    cs.commit().unwrap();
    let mut b = body(&export(&two).unwrap());
    let at = (0..b.len() - 8).rev().find(|&i| b[i..i + 8] == 2u64.to_be_bytes()).unwrap();
    b[at..at + 8].copy_from_slice(&1u64.to_be_bytes());
    assert!(corrupt(import(&open::<E>(), &seal(b))).contains("does not grow"));
    // A sequence the importer will not replay commit by commit.
    let far = (MAX_REPLAY_SEQ + 1).to_be_bytes();
    let mut b = good.clone();
    b[CREATED_SEQ_AT..CREATED_SEQ_AT + 8].copy_from_slice(&far);
    b[COMMIT_SEQ_AT..COMMIT_SEQ_AT + 8].copy_from_slice(&far);
    let err = import(&open::<E>(), &seal(b)).unwrap_err();
    assert!(matches!(err, UnitError::InvalidArgument(_)), "{err:?}");
}

#[test]
fn structure_errors_emulated_log() {
    structure_errors::<EmuLog>();
}

#[test]
fn structure_errors_kerno_db() {
    structure_errors::<KernoDb>();
}

fn target_rules<E: TxnEngine>() {
    let src = open::<E>();
    populate(&src);
    let bytes = export(&src).unwrap();
    // A database with a commit is not new.
    let used = open::<E>();
    used.create_unit(new_unit("doc", b"y"), ChangeMeta::default()).unwrap();
    let err = import(&used, &bytes).unwrap_err();
    assert!(matches!(&err, UnitError::InvalidArgument(m) if m.contains("new database")), "{err:?}");
    // Importing twice is the same case.
    let dst = open::<E>();
    import(&dst, &bytes).unwrap();
    assert!(matches!(import(&dst, &bytes), Err(UnitError::InvalidArgument(_))));
    assert_eq!(export(&dst).unwrap(), bytes);
    // Another default policy would change the meaning of the history.
    let other = open_with::<E>(HistoryPolicy::Audit);
    assert!(matches!(import(&other, &bytes), Err(UnitError::InvalidArgument(_))));
    assert_eq!(other.snapshot().unwrap().seq(), CommitSeq(0));
}

#[test]
fn target_rules_emulated_log() {
    target_rules::<EmuLog>();
}

#[test]
fn target_rules_kerno_db() {
    target_rules::<KernoDb>();
}

/// Policies, units and relations come in strictly increasing order; a file
/// that swaps two records of the same size is refused.
fn order_errors<E: TxnEngine>() {
    let src = open::<E>();
    src.set_policy("aa", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    src.set_policy("bb", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
    let a = cs.create(new_unit("doc", b"x")).unwrap();
    let b = cs.create(new_unit("doc", b"y")).unwrap();
    cs.create_relation(a, b, "r", RelationProvenance::Declared).unwrap();
    cs.create_relation(b, a, "r", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let good = body(&export(&src).unwrap());
    import(&open::<E>(), &seal(good.clone())).unwrap();

    let (policies, unit, relation) = (16 + 1 + 4, 140, 68);
    // Policy records, the token count (no tokens here), then the unit count.
    let units = policies + 2 * 7 + 4 + 4;
    let rels = units + 2 * unit + 4;
    assert_eq!(good.len(), rels + 2 * relation);
    let swapped = |start: usize, len: usize| {
        let mut b = good.clone();
        let (x, y) = (good[start..start + len].to_vec(), good[start + len..start + 2 * len].to_vec());
        b[start..start + len].copy_from_slice(&y);
        b[start + len..start + 2 * len].copy_from_slice(&x);
        seal(b)
    };
    assert!(corrupt(import(&open::<E>(), &swapped(policies, 7))).contains("policies"));
    assert!(corrupt(import(&open::<E>(), &swapped(units, unit))).contains("units"));
    assert!(corrupt(import(&open::<E>(), &swapped(rels, relation))).contains("relations"));
    // A repeated key is as wrong as a swapped one: "bb" becomes "aa", and the
    // second unit and relation take the id of the first.
    let mut b = good.clone();
    b[policies + 7 + 4..policies + 7 + 6].copy_from_slice(b"aa");
    assert!(corrupt(import(&open::<E>(), &seal(b))).contains("policies"));
    let mut b = good.clone();
    b.copy_within(units..units + 16, units + unit);
    assert!(corrupt(import(&open::<E>(), &seal(b))).contains("units"));
    let mut b = good.clone();
    b.copy_within(rels..rels + 16, rels + relation);
    assert!(corrupt(import(&open::<E>(), &seal(b))).contains("relations"));

    // A relation to a unit the file does not hold.
    let mut b = good.clone();
    b[rels + 47] = 9;
    assert!(corrupt(import(&open::<E>(), &seal(b))).contains("not in the file"));
    // A declared relation with a confidence no writer gives it.
    let mut b = good.clone();
    b[rels + relation - 1] = 1;
    assert!(corrupt(import(&open::<E>(), &seal(b))).contains("outside what a writer produces"));
}

#[test]
fn order_errors_emulated_log() {
    order_errors::<EmuLog>();
}

#[test]
fn order_errors_kerno_db() {
    order_errors::<KernoDb>();
}

/// What the file passes by itself but the database refuses is caught by the
/// verifier that closes the import: a relation to an archived unit.
fn verifier_closes_import<E: TxnEngine>() {
    let src = open::<E>();
    let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
    let a = cs.create(new_unit("doc", b"x")).unwrap();
    let b = cs.create(new_unit("doc", b"y")).unwrap();
    cs.commit().unwrap();
    let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(b, upd(b"y"), None).unwrap();
    cs.commit().unwrap();
    let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(a, b, "r", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let good = body(&export(&src).unwrap());
    import(&open::<E>(), &seal(good.clone())).unwrap();
    // Revision 2 of `b` becomes an Archive of the same content.
    let mut pattern = 2u64.to_be_bytes().to_vec();
    pattern.extend([RevisionKind::Update.to_byte(), 1]);
    pattern.extend(1u64.to_be_bytes());
    let at = good.windows(pattern.len()).position(|w| w == pattern).unwrap();
    let mut bad = good.clone();
    bad[at + 8] = RevisionKind::Archive.to_byte();
    match import(&open::<E>(), &seal(bad)) {
        Err(UnitError::Storage(CoreError::Corruption(m))) => assert!(m.starts_with("UNIT-36"), "{m}"),
        other => panic!("expected the verifier to refuse, got {other:?}"),
    }
}

#[test]
fn verifier_closes_import_emulated_log() {
    verifier_closes_import::<EmuLog>();
}

#[test]
fn verifier_closes_import_kerno_db() {
    verifier_closes_import::<KernoDb>();
}

/// Fields of one unit with two revisions (`x`, then `y`), tampered one at a
/// time: each refusal names the rule the importer's own parsing applies.
fn revision_errors<E: TxnEngine>() {
    let src = open::<E>();
    let (id, _) = src.create_unit(new_unit("doc", b"x"), ChangeMeta::default()).unwrap();
    let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, upd(b"y"), None).unwrap();
    cs.commit().unwrap();
    let good = body(&export(&src).unwrap());
    import(&open::<E>(), &seal(good.clone())).unwrap();

    // Unit fields from `created_seq`; then the revisions. The second one
    // carries a parent, so it is eight bytes longer than the first.
    let unit = CREATED_SEQ_AT;
    let (oldest, head, count, r1) = (unit + 8 + 8, unit + 8 + 8 + 8, unit + 8 + 8 + 8 + 8, unit + 36);
    let r1_len = (good.len() - 4 - r1 - 8) / 2;
    let r2 = r1 + r1_len;
    assert_eq!(good[r1 + 8], RevisionKind::Create.to_byte());
    assert_eq!(good[r2 + 8], RevisionKind::Update.to_byte());
    assert_eq!(good[r2 + 9], 1, "the second revision has a parent");
    assert_eq!(good[r2 + 10..r2 + 18], 1u64.to_be_bytes());
    assert_eq!(good[r2 + 18..r2 + 26], 2u64.to_be_bytes());
    let r1_restored = r1 + 67;
    let (r2_alg, r2_len, r2_restored) = (r2 + 38, r2 + 71, r2 + 75);
    assert_eq!(good[r1_restored], 0);
    assert_eq!(good[r2_restored], 0);
    assert_eq!(good[r2_len..r2_len + 4], 1u32.to_be_bytes());

    let put = |at: usize, v: &[u8]| {
        let mut b = good.clone();
        b[at..at + v.len()].copy_from_slice(v);
        b
    };
    let insert = |mut b: Vec<u8>, at: usize, v: &[u8]| {
        b.splice(at..at, v.iter().copied());
        b
    };
    let refused = |b: Vec<u8>, what: &str| {
        let m = corrupt(import(&open::<E>(), &seal(b)));
        assert!(m.contains(what), "{m} does not say {what:?}");
    };

    refused(put(r2, &3u64.to_be_bytes()), "numbered");
    refused(put(r2_alg, &[9]), "hash algorithm");
    refused(put(r1 + 10, &0u64.to_be_bytes()), "commit_seq 0");
    refused(put(r1 + 8, &[RevisionKind::Update.to_byte()]), "at this revision");
    refused(put(r2 + 8, &[RevisionKind::Create.to_byte()]), "at this revision");
    refused(put(r2 + 10, &5u64.to_be_bytes()), "parent is not the previous");
    // A parent on the first revision (a flag and eight bytes).
    refused(insert(put(r1 + 9, &[1]), r1 + 10, &[0; 8]), "has a parent");
    // `Restore` without a source, and a source without `Restore`.
    refused(put(r2 + 8, &[RevisionKind::Restore.to_byte()]), "restored_from and kind disagree");
    refused(insert(put(r2_restored, &[1]), r2_restored + 1, &1u64.to_be_bytes()), "restored_from and kind disagree");
    // Unit level: the oldest revision and the revision count.
    let mut b = put(oldest, &0u64.to_be_bytes());
    b[head..head + 8].copy_from_slice(&1u64.to_be_bytes());
    refused(b, "do not cover");
    // A head below the oldest with a count that would wrap to match.
    let mut b = put(oldest, &2u64.to_be_bytes());
    b[head..head + 8].copy_from_slice(&1u64.to_be_bytes());
    b[count..count + 4].copy_from_slice(&0u32.to_be_bytes());
    refused(b, "do not cover");
    refused(put(count, &3u32.to_be_bytes()), "do not cover");
    refused(put(head, &3u64.to_be_bytes()), "do not cover");
    // A payload that is not the declared one: length, and content.
    let m = rule::<E>(&seal(put(r2_len, &2u32.to_be_bytes())));
    assert!(m.starts_with("UNIT-27"), "{m}");
    let m = rule::<E>(&seal(put(r2 - 1, b"z")));
    assert!(m.starts_with("UNIT-28"), "{m}");

    // A history that starts after revision 1 (the oldest one left the
    // retention) is read; its first revision must come after the creation.
    let mut cut = good.clone();
    cut.drain(r1..r2);
    cut[oldest..oldest + 8].copy_from_slice(&2u64.to_be_bytes());
    cut[count..count + 4].copy_from_slice(&1u32.to_be_bytes());
    for created in [2u64, 3] {
        let mut early = cut.clone();
        early[unit..unit + 8].copy_from_slice(&created.to_be_bytes());
        refused(early, "disagrees with its first revision");
    }
    let mut zero = cut;
    zero[unit..unit + 8].copy_from_slice(&0u64.to_be_bytes());
    refused(zero, "disagrees with its first revision");
}

#[test]
fn revision_errors_emulated_log() {
    revision_errors::<EmuLog>();
}

#[test]
fn revision_errors_kerno_db() {
    revision_errors::<KernoDb>();
}

/// The fields of one relation record, tampered one at a time: its type, its
/// sequence, its provenance kind, what a provenance may carry, and both ends.
fn relation_errors<E: TxnEngine>() {
    // A file with two units and one relation; the record is the file's tail.
    let file = |provenance: RelationProvenance| {
        let src = open::<E>();
        let mut cs = src.begin_changeset(ChangeMeta::default()).unwrap();
        let a = cs.create(new_unit("doc", b"x")).unwrap();
        let b = cs.create(new_unit("doc", b"y")).unwrap();
        cs.create_relation(a, b, "r", provenance).unwrap();
        cs.commit().unwrap();
        body(&export(&src).unwrap())
    };
    // id 16, from 16, to 16, type 4 + 1, created_seq 8, kind 1, origin 4 + n, confidence 2.
    let (from, to, ty, seq, kind, origin) = (16, 32, 48, 53, 61, 62);
    let refused = |b: Vec<u8>, what: &str| {
        let m = corrupt(import(&open::<E>(), &seal(b)));
        assert!(m.contains(what), "{m} does not say {what:?}");
    };
    let writer = "outside what a writer produces";

    let declared = file(RelationProvenance::Declared);
    let r = declared.len() - 68;
    import(&open::<E>(), &seal(declared.clone())).unwrap();
    let put = |b: &[u8], at: usize, v: &[u8]| {
        let mut b = b.to_vec();
        b[at..at + v.len()].copy_from_slice(v);
        b
    };
    refused(put(&declared, r + ty + 4, &[0]), writer);
    refused(put(&declared, r + seq, &0u64.to_be_bytes()), writer);
    refused(put(&declared, r + kind, &[9]), "provenance byte");
    refused(put(&declared, r + from + 15, &[9]), "not in the file");
    refused(put(&declared, r + to + 15, &[9]), "not in the file");
    // An origin on a declared relation: the length, then one byte of it.
    let mut b = put(&declared, r + origin, &1u32.to_be_bytes());
    b.insert(r + origin + 4, b'o');
    refused(b, writer);

    // Every kind but the inferred one carries no confidence; the inferred
    // one carries any.
    for p in [
        RelationProvenance::Imported { source: b"w".to_vec() },
        RelationProvenance::Algorithmic { origin: b"w".to_vec() },
    ] {
        let f = file(p);
        import(&open::<E>(), &seal(f.clone())).unwrap();
        refused(put(&f, f.len() - 1, &[1]), writer);
    }
    let f = file(RelationProvenance::Inferred { origin: b"w".to_vec(), confidence_permille: 640 });
    import(&open::<E>(), &seal(f)).unwrap();
}

#[test]
fn relation_errors_emulated_log() {
    relation_errors::<EmuLog>();
}

#[test]
fn relation_errors_kerno_db() {
    relation_errors::<KernoDb>();
}

/// Policy names: the longest one a database takes imports; an empty one and
/// a longer one are refused; a header-sized input is read as a header.
fn policy_errors<E: TxnEngine>() {
    let longest = "p".repeat(57);
    let src = open::<E>();
    src.set_policy(&longest, HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    let good = export(&src).unwrap();
    let dst = open::<E>();
    import(&dst, &good).unwrap();
    assert_eq!(export(&dst).unwrap(), good);

    // One policy "a": the count, then length, name, policy byte.
    let one = open::<E>();
    one.set_policy("a", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    let good = body(&export(&one).unwrap());
    let at = 16 + 1 + 4;
    assert_eq!(good[at..at + 5], [0, 0, 0, 1, b'a']);
    let mut empty = good.clone();
    empty[at..at + 4].copy_from_slice(&0u32.to_be_bytes());
    empty.remove(at + 4);
    assert!(corrupt(import(&open::<E>(), &seal(empty))).contains("outside 1..=57"));
    let mut long = good;
    long[at..at + 4].copy_from_slice(&58u32.to_be_bytes());
    long.splice(at + 4..at + 5, "p".bytes().cycle().take(58));
    assert!(corrupt(import(&open::<E>(), &seal(long))).contains("outside 1..=57"));

    // Exactly a header and a digest: long enough to be read, so the magic
    // is what it fails on.
    let m = corrupt(import(&open::<E>(), &[0; 16 + DIGEST_LEN]));
    assert!(m.contains("magic"), "{m}");
}

#[test]
fn policy_errors_emulated_log() {
    policy_errors::<EmuLog>();
}

#[test]
fn policy_errors_kerno_db() {
    policy_errors::<KernoDb>();
}
