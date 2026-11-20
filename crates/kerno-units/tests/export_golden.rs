//! Golden files of the frozen export format (`docs/file-format/export-v1.md`).
//!
//! `tests/golden/export-v1/*.kexp` are committed bytes. A change of one byte
//! the exporter writes, or of one byte the importer accepts, turns a test here
//! red; that is the point of a frozen format. Fixing it means a new
//! `format_version`, regenerated files and an ADR, not editing a file to match.
//!
//! * `empty` — an empty database, also written here by hand from the spec.
//! * `full` — every section: both kinds of policy, tokens (one unit without
//!   one, one deleted), every revision kind, a tombstone, actors and sources
//!   present and absent, one relation of each provenance.
//!
//! The tokens come from the operating system's random source when a database
//! issues them, so `full` is built from a database export with the token
//! section filled in by [`with_tokens`]; importing it restores them.
//!
//! To regenerate after an approved format change, run with
//! `KERNO_REGEN_GOLDEN=1`: it rewrites the files and the run fails, so no
//! regeneration passes CI by accident.

use std::path::{Path, PathBuf};

use kerno_core::Error as CoreError;
use kerno_core::contract::{Options, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::export::{DIGEST_LEN, EXPORT_MAGIC, EXPORT_VERSION, export};
use kerno_units::ids::PublicId;
use kerno_units::import::import;
use kerno_units::*;
use std::sync::Arc;

type EmuLog = Emulated<LogEngine>;

const EMPTY: &[u8] = include_bytes!("golden/export-v1/empty.kexp");
const FULL: &[u8] = include_bytes!("golden/export-v1/full.kexp");

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

/// The tokens of units 1 and 3 in `full`: 16 bytes each, not all equal.
const TOKENS: [(u8, [u8; 16]); 2] = [
    (1, [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F]),
    (3, [0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F]),
];

fn open<E: TxnEngine>() -> UnitDb<E> {
    open_from(1)
}

/// A database whose next generated unit id is `first_id`.
fn open_from<E: TxnEngine>(first_id: u64) -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(first_id)));
    c.engine = Options::small_for_tests();
    UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open")
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

/// Units 1..=4 (`doc`, `doc`, `log`, `doc`), every revision kind, a gap in the
/// commit sequence, both policy records, one relation of each provenance.
fn populate<E: TxnEngine>(db: &UnitDb<E>) {
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
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(a, c, "cites", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta { actor: Some(b"eve".to_vec()), ..ChangeMeta::default() }).unwrap();
    cs.delete(d, None).unwrap();
    cs.commit().unwrap();
    db.set_policy("doc", HistoryPolicy::Versioned, ChangeMeta::default()).unwrap();
}

fn seal(mut body: Vec<u8>) -> Vec<u8> {
    let digest = blake3::hash(&body);
    body.extend_from_slice(digest.as_bytes());
    body
}

fn body(bytes: &[u8]) -> Vec<u8> {
    bytes[..bytes.len() - DIGEST_LEN].to_vec()
}

/// Where the token count starts: after the header, the default policy byte and
/// the policy records (export-v1.md §4, sections 1 and 2).
fn tokens_at(bytes: &[u8]) -> usize {
    let mut r = Spec { b: &bytes[16 + 1..] };
    let n = r.u32();
    for _ in 0..n {
        r.bytes();
        r.u8();
    }
    bytes.len() - r.b.len()
}

/// The export of `populate` with the token section filled in.
fn with_tokens(exported: &[u8]) -> Vec<u8> {
    let at = tokens_at(exported);
    let b = body(exported);
    assert_eq!(b[at..at + 4], [0; 4], "the source has no tokens");
    let mut out = b[..at].to_vec();
    out.extend_from_slice(&(TOKENS.len() as u32).to_be_bytes());
    for (unit, token) in TOKENS {
        let mut id = [0u8; 16];
        id[15] = unit;
        out.extend_from_slice(&id);
        out.extend_from_slice(&token);
    }
    out.extend_from_slice(&b[at + 4..]);
    seal(out)
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/export-v1")
}

fn full_from_code() -> Vec<u8> {
    let db = open::<EmuLog>();
    populate(&db);
    with_tokens(&export(&db).unwrap())
}

#[test]
fn regenerate_only_on_request() {
    if std::env::var_os("KERNO_REGEN_GOLDEN").is_none() {
        return;
    }
    let empty = export(&open::<EmuLog>()).unwrap();
    std::fs::write(golden_dir().join("empty.kexp"), empty).unwrap();
    std::fs::write(golden_dir().join("full.kexp"), full_from_code()).unwrap();
    panic!("golden files rewritten: review the diff, then run without KERNO_REGEN_GOLDEN");
}

// -------------------------------------------------- a decoder written from the spec

/// Reads the spec's encodings and nothing of the crate's own decoder.
struct Spec<'a> {
    b: &'a [u8],
}

impl<'a> Spec<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let (h, t) = self.b.split_at(n);
        self.b = t;
        h
    }
    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take(4).try_into().unwrap())
    }
    fn u64(&mut self) -> u64 {
        u64::from_be_bytes(self.take(8).try_into().unwrap())
    }
    fn bytes(&mut self) -> &'a [u8] {
        let n = self.u32() as usize;
        self.take(n)
    }
    fn opt_u64(&mut self) -> Option<u64> {
        (self.u8() == 1).then(|| self.u64())
    }
    fn opt_bytes(&mut self) -> Option<&'a [u8]> {
        (self.u8() == 1).then(|| self.bytes())
    }
}

/// `(rev, kind, has_payload, actor, source)` of one revision.
type Rev = (u64, u8, bool, Option<Vec<u8>>, Option<Vec<u8>>);

/// What the decoder read: counts and the few values the tests pin.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    default_policy: u8,
    policies: Vec<(Vec<u8>, u8)>,
    tokens: Vec<([u8; 16], [u8; 16])>,
    /// Per unit: `(type, oldest, head, revisions as (rev, kind, has_payload, actor, source))`.
    units: Vec<(Vec<u8>, u64, u64, Vec<Rev>)>,
    /// `(relation_type, provenance, origin, confidence)` in file order.
    relations: Vec<(Vec<u8>, u8, Vec<u8>, u16)>,
}

fn decode(file: &[u8]) -> Shape {
    assert_eq!(&file[..8], b"KUNITEXD");
    assert_eq!(file[8..12], 1u32.to_be_bytes());
    assert_eq!(file[12..16], [0; 4], "flags and reserved bytes");
    let (content, digest) = file.split_at(file.len() - 32);
    assert_eq!(blake3::hash(content).as_bytes(), digest);
    let mut r = Spec { b: &content[16..] };
    let default_policy = r.u8();
    let policies = (0..r.u32()).map(|_| (r.bytes().to_vec(), r.u8())).collect();
    let tokens = (0..r.u32()).map(|_| (r.take(16).try_into().unwrap(), r.take(16).try_into().unwrap())).collect();
    let units = (0..r.u32())
        .map(|_| {
            r.take(16);
            let unit_type = r.bytes().to_vec();
            let (_created_seq, _created_at, oldest, head) = (r.u64(), r.u64(), r.u64(), r.u64());
            let revs = (0..r.u32())
                .map(|_| {
                    let rev = r.u64();
                    let kind = r.u8();
                    r.opt_u64(); // parent
                    r.u64(); // commit_seq
                    r.u64(); // created_at
                    r.u32(); // schema_version
                    r.take(1 + 32); // content_hash
                    r.u32(); // payload_len
                    r.opt_u64(); // restored_from
                    r.bytes(); // metadata
                    let actor = r.opt_bytes().map(<[u8]>::to_vec);
                    let source = r.opt_bytes().map(<[u8]>::to_vec);
                    let payload = r.opt_bytes();
                    (rev, kind, payload.is_some(), actor, source)
                })
                .collect();
            (unit_type, oldest, head, revs)
        })
        .collect();
    let relations = (0..r.u32())
        .map(|_| {
            r.take(48); // id, from, to
            let relation_type = r.bytes().to_vec();
            r.u64(); // created_seq
            let provenance = r.u8();
            let origin = r.bytes().to_vec();
            let confidence = u16::from_be_bytes(r.take(2).try_into().unwrap());
            (relation_type, provenance, origin, confidence)
        })
        .collect();
    assert!(r.b.is_empty(), "nothing between the last section and the digest");
    Shape { default_policy, policies, tokens, units, relations }
}

// ------------------------------------------------------------------------ tests

/// The empty export written from the spec: header, "no layer mark", four
/// zero counts, the digest. Not produced by the exporter.
#[test]
fn empty_is_what_the_spec_says() {
    let mut by_hand = Vec::new();
    by_hand.extend_from_slice(b"KUNITEXD");
    by_hand.extend_from_slice(&[0, 0, 0, 1]); // format_version 1
    by_hand.extend_from_slice(&[0, 0, 0, 0]); // flags 0, reserved 3 x 0
    by_hand.push(0xFF); // no default policy
    by_hand.extend_from_slice(&[0; 16]); // policies, tokens, units, relations: 0
    let by_hand = seal(by_hand);
    assert_eq!(EMPTY, by_hand.as_slice());
    assert_eq!(&EMPTY[..8], &EXPORT_MAGIC);
    assert_eq!(EXPORT_VERSION, 1);
}

fn exports_the_golden_bytes<E: TxnEngine>() {
    assert_eq!(export(&open::<E>()).unwrap(), EMPTY);
    let db = open::<E>();
    populate(&db);
    assert_eq!(with_tokens(&export(&db).unwrap()), FULL, "the exporter no longer writes the frozen bytes");
    // Without the tokens the golden is exactly what the exporter writes.
    let mut plain = body(FULL);
    let at = tokens_at(FULL);
    plain.drain(at + 4..at + 4 + TOKENS.len() * 32);
    plain[at..at + 4].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(seal(plain), export(&db).unwrap());
}

#[test]
fn exports_the_golden_bytes_emulated_log() {
    exports_the_golden_bytes::<EmuLog>();
}

#[test]
fn exports_the_golden_bytes_kerno_db() {
    exports_the_golden_bytes::<KernoDb>();
}

/// The frozen file is a valid file of every engine today: importing it and
/// exporting again gives the same bytes, and the tokens come back.
fn imports_the_golden_files<E: TxnEngine>() {
    let dst = open::<E>();
    let report = import(&dst, EMPTY).unwrap();
    assert_eq!(report, Default::default());
    assert_eq!(export(&dst).unwrap(), EMPTY);

    // Ids it generates itself must not meet the imported ones.
    let dst = open_from::<E>(100);
    let report = import(&dst, FULL).unwrap();
    assert_eq!((report.units, report.relations, report.policies, report.tokens), (4, 5, 2, 2));
    assert_eq!(export(&dst).unwrap(), FULL);
    for (unit, token) in TOKENS {
        let mut id = [0u8; 16];
        id[15] = unit;
        let (id, token) = (UnitId(id), PublicId::from_bytes(token));
        assert_eq!(dst.public_id(id).unwrap(), Some(token));
        assert_eq!(dst.resolve_public_id(&token).unwrap(), Some(id));
    }
    // The imported database takes further writes after the replayed sequences.
    // The file names sequences up to 7; the next write lands after them.
    let (_, info) = dst.create_unit(new_unit("doc", b"after"), ChangeMeta::default()).unwrap();
    assert!(info.commit_seq.0 > 7, "{:?}", info.commit_seq);
}

#[test]
fn imports_the_golden_files_emulated_log() {
    imports_the_golden_files::<EmuLog>();
}

#[test]
fn imports_the_golden_files_kerno_db() {
    imports_the_golden_files::<KernoDb>();
}

/// A reader written from the spec reads the golden file and finds what the
/// fixture put in it.
#[test]
fn spec_decoder_reads_the_golden_file() {
    assert_eq!(
        decode(EMPTY),
        Shape { default_policy: 0xFF, policies: vec![], tokens: vec![], units: vec![], relations: vec![] }
    );
    let s = decode(FULL);
    assert_eq!(s.default_policy, HistoryPolicy::Versioned.to_byte());
    // In unit type order.
    assert_eq!(
        s.policies,
        vec![(b"doc".to_vec(), HistoryPolicy::Versioned.to_byte()), (b"log".to_vec(), HistoryPolicy::None.to_byte())]
    );
    assert_eq!(s.tokens.len(), 2);
    assert_eq!(s.tokens[0].1, TOKENS[0].1);
    assert_eq!(s.tokens[1].1, TOKENS[1].1);
    let kinds = |u: usize| s.units[u].3.iter().map(|r| r.1).collect::<Vec<_>>();
    // Unit 1: create, update, restore. Unit 2: create, archive, unarchive.
    assert_eq!(kinds(0), [1, 2, 6]);
    assert_eq!(kinds(1), [1, 3, 4]);
    // Unit 3 (`log`, policy None) keeps its head revision only.
    assert_eq!((s.units[2].1, s.units[2].2, kinds(2)), (3, 3, vec![2]));
    // Unit 4 was deleted: its last revision is a tombstone without a payload.
    assert_eq!(kinds(3), [1, 5]);
    assert!(!s.units[3].3[1].2);
    // Actor and source are both present, both absent, or only the actor.
    assert_eq!(s.units[0].3[0].3.as_deref(), Some(&b"ann"[..]));
    assert_eq!(s.units[0].3[0].4.as_deref(), Some(&b"cli"[..]));
    assert_eq!(s.units[0].3[2].3, None);
    assert_eq!((s.units[3].3[1].3.as_deref(), s.units[3].3[1].4.as_deref()), (Some(&b"eve"[..]), None));
    // One relation of each provenance, the inferred one with its confidence.
    let provenances: Vec<_> = s.relations.iter().map(|r| (r.1, r.3)).collect();
    for want in [(0u8, 0u16), (1, 0), (2, 0), (3, 640)] {
        assert!(provenances.contains(&want), "{want:?} in {provenances:?}");
    }
    assert_eq!(s.relations.len(), 5);
}

/// Every other header a reader of version 1 meets.
fn refuses_other_headers<E: TxnEngine>() {
    let refused = |bytes: Vec<u8>| import(&open::<E>(), &bytes).unwrap_err();
    let incompatible = |e: &UnitError, what: &str| {
        assert!(matches!(e, UnitError::Storage(CoreError::Incompatible(m)) if m.contains(what)), "{e:?}");
    };
    // The pre-freeze draft: same version, `flags = 1`.
    let mut draft = body(FULL);
    draft[12] = 1;
    incompatible(&refused(seal(draft)), "pre-freeze draft export");
    // Versions 0 (no tokens) and 2 (not yet defined).
    for v in [0u32, 2, u32::MAX] {
        let mut other = body(FULL);
        other[8..12].copy_from_slice(&v.to_be_bytes());
        incompatible(&refused(seal(other)), "format_version");
    }
    // A defined bit that a future version may give meaning to is not ignored.
    let mut flagged = body(FULL);
    flagged[12] = 2;
    assert!(matches!(refused(seal(flagged)), UnitError::Storage(CoreError::Corruption(_))));
    // A digest that does not match.
    let mut torn = FULL.to_vec();
    let last = torn.len() - 1;
    torn[last] ^= 1;
    assert!(matches!(refused(torn), UnitError::Storage(CoreError::Corruption(_))));
}

#[test]
fn refuses_other_headers_emulated_log() {
    refuses_other_headers::<EmuLog>();
}

#[test]
fn refuses_other_headers_kerno_db() {
    refuses_other_headers::<KernoDb>();
}
