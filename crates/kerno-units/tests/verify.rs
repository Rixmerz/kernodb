//! E3-07: `verify_units` and damage (`etapa3-design.md` §8.4, §8.5, §11;
//! `units-v1.md` §5.2). The generic suites run over `Emulated<LogEngine>`
//! and `KernoDb`.
//!
//! * `clean_scenario` — a store with every kind of revision, every history
//!   policy, a policy change and a retention change set verifies clean after
//!   every step, and no revision changes between consecutive snapshots
//!   (IU-1, `verify_immutable`).
//! * `damage_detected` — each kind of damage, injected with valid engine
//!   checksums (a logical patch of one record through the engine, the
//!   equivalent of an SST patched with its CRCs recomputed), is reported
//!   with its `UNIT-n` rule, its error is `Corruption` or
//!   `IncompatibleLayer`, and the reads that touch it fail with the same
//!   class (the content hash is checked on read, IU-5).
//! * `media_damage` — bit flips and truncations of the files of a units
//!   store (§8.4): every error is `Corruption`, `Io` or `Poisoned` (I-08),
//!   and a store that still opens reads exactly what was written or
//!   reports the damage.
//! * `verify_writes_nothing` — `verify_units`, `verify_engine` and the
//!   refused opens of a damaged mark leave the store unchanged; on `KernoDb`
//!   opened read-only, the directory too, byte for byte (I-18).
//!
//! The other suites (`unit_db`, `history`, `policies`, `retention`) assert
//! that `verify_units` finds nothing after their workloads and after every
//! crash of their sweeps.

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_storage::{AccessMode, Config, KernoDb};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{DirImage, OpenMode, SimVfs, assert_dir_unchanged};
use kerno_units::codec::{ChangeSetRecord, CsMember, Key, LayerMeta, RevisionMeta, UnitHead};
use kerno_units::*;

const DIR: &str = "/db";
const T0: Timestamp = Timestamp(1_700_000_000_000_000);

type Raw = BTreeMap<Vec<u8>, Vec<u8>>;

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::stepping(T0, 1_000_000)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn meta() -> ChangeMeta {
    ChangeMeta::default()
}

fn new_unit(t: &str, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: 1, metadata: b"m".to_vec(), payload: payload.to_vec() }
}

fn update<E: TxnEngine>(db: &UnitDb<E>, id: UnitId, p: &[u8]) {
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(id, UnitUpdate { payload: p.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

fn raw_of<S: Snapshot>(s: &S) -> Raw {
    let mut out = Raw::new();
    loop {
        let last = out.keys().next_back().cloned();
        let lo = last.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
        let page = s.scan(lo, Bound::Unbounded, 128).expect("scan");
        let n = page.len();
        out.extend(page);
        if n < 128 {
            return out;
        }
    }
}

#[track_caller]
fn assert_clean(r: &UnitVerifyReport, ctx: &str) {
    assert!(r.is_clean(), "{ctx}: verify_units found {:?}", r.findings);
}

fn k(key: Key) -> Vec<u8> {
    key.encode().unwrap()
}

// ------------------------------------------------------------ the base store

/// The units of the base store.
struct Ids {
    /// "doc": Create, Update, Archive, Unarchive, Restore(1).
    a: UnitId,
    /// "doc": Create, Delete.
    b: UnitId,
    /// "aud" (policy Audit): Create, Update; rev 1 lost its payload.
    c: UnitId,
    /// "non" (policy None): Create, Update; rev 1 is gone.
    d: UnitId,
    /// "ret": 4 revisions, retention kept the last 2.
    e: UnitId,
}

struct Base {
    image: DirImage,
    ids: Ids,
    raw: Raw,
    /// `REV_META`/`REV_PAYLOAD` of `d` rev 1 before policy `None` removed them.
    d_rev1: (Vec<u8>, Vec<u8>),
}

/// Builds the base store, checking after every step that it verifies clean
/// and that no revision changed (IU-1).
fn base<E: TxnEngine>(seed: u64) -> Base {
    let disk = SimVfs::new(seed);
    let db = UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), &cfg()).expect("open");
    assert_clean(&db.verify_units().unwrap(), "empty store");
    let mut prev = db.engine().snapshot().unwrap();
    let mut step = |db: &UnitDb<E>, what: &str| {
        let now = db.engine().snapshot().unwrap();
        assert_clean(&db.verify_units().unwrap(), what);
        let changed = verify_immutable(&prev, &now).unwrap();
        assert!(changed.is_empty(), "{what}: {changed:?}");
        prev = now;
    };
    db.set_policy("aud", HistoryPolicy::Audit, meta()).unwrap();
    db.set_policy("non", HistoryPolicy::None, meta()).unwrap();
    step(&db, "policies");
    let (a, _) = db.create_unit(new_unit("doc", &[1; 10]), meta()).unwrap();
    update(&db, a, &[2; 10]);
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.archive(a, None).unwrap();
    let b = cs.create(new_unit("doc", &[3; 10])).unwrap();
    cs.commit().unwrap();
    step(&db, "archive");
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.unarchive(a, None).unwrap();
    cs.delete(b, None).unwrap();
    cs.commit().unwrap();
    db.restore(a, RevNo(1), None, meta()).unwrap();
    step(&db, "restore");
    let (c, _) = db.create_unit(new_unit("aud", &[4; 10]), meta()).unwrap();
    update(&db, c, &[5; 10]);
    let (d, _) = db.create_unit(new_unit("non", &[6; 10]), meta()).unwrap();
    let snap = db.engine().snapshot().unwrap();
    let d_rev1 = (
        snap.get(&k(Key::RevMeta { unit: d, rev: RevNo(1) })).unwrap().unwrap(),
        snap.get(&k(Key::RevPayload { unit: d, rev: RevNo(1) })).unwrap().unwrap(),
    );
    drop(snap);
    update(&db, d, &[7; 10]);
    step(&db, "policies applied");
    let (e, _) = db.create_unit(new_unit("ret", &[8; 10]), meta()).unwrap();
    for i in 9..12 {
        update(&db, e, &[i; 10]);
    }
    let r = db
        .run_retention(
            &RetentionPolicy::all(RetentionRule::KeepAll).with_type("ret", RetentionRule::KeepLast(2)),
            RetentionBudget::default(),
        )
        .unwrap();
    assert!(r.change_set.is_some() && r.revisions_pruned == 2, "{r:?}");
    step(&db, "retention");
    let report = db.verify_units().unwrap();
    assert_eq!(
        (report.counts.units, report.counts.revisions, report.counts.payloads),
        (5, 5 + 2 + 2 + 1 + 2, 5 + 1 + 1 + 1 + 2),
        "{:?}",
        report.counts
    );
    assert_eq!(report.counts.layer_records, 3);
    assert_eq!(report.counts.unknown, 0);
    let raw = raw_of(&db.engine().snapshot().unwrap());
    db.close().unwrap();
    // A reopened store verifies the same.
    let db = UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), &cfg()).unwrap();
    assert_eq!(db.verify_units().unwrap(), report);
    db.close().unwrap();
    Base { image: disk.dump(), ids: Ids { a, b, c, d, e }, raw, d_rev1 }
}

fn clean_scenario<E: TxnEngine>() {
    base::<E>(70);
}

// ------------------------------------------------------------ logical damage

/// What a damage does: puts (`Some`) and deletions (`None`).
type Patch = Vec<(Vec<u8>, Option<Vec<u8>>)>;

fn seq_of(raw: &Raw, unit: UnitId, rev: u64) -> CommitSeq {
    RevisionMeta::decode(RevNo(rev), &raw[&k(Key::RevMeta { unit, rev: RevNo(rev) })]).unwrap().commit_seq
}

fn rev_meta(raw: &Raw, unit: UnitId, rev: u64) -> RevisionMeta {
    RevisionMeta::decode(RevNo(rev), &raw[&k(Key::RevMeta { unit, rev: RevNo(rev) })]).unwrap()
}

fn put_meta(unit: UnitId, rev: u64, m: &RevisionMeta) -> (Vec<u8>, Option<Vec<u8>>) {
    (k(Key::RevMeta { unit, rev: RevNo(rev) }), Some(m.encode(RevNo(rev)).unwrap()))
}

/// The damages of §8.4 and of every rule of `units-v1.md` §5–§5.2: name,
/// patch, and the rule the verifier must report.
fn damages(b: &Base) -> Vec<(&'static str, Patch, &'static str)> {
    let Ids { a, b: bu, c, d, e } = b.ids;
    let raw = &b.raw;
    let mark = k(Key::LayerMeta { name: "layer".into() });
    let pay = |u, r| k(Key::RevPayload { unit: u, rev: RevNo(r) });
    let head_key = |u| k(Key::UnitHead { unit: u });
    let mut flipped = raw[&pay(a, 2)].clone();
    flipped[3] ^= 0x10;
    let mut head_a = UnitHead::decode(&raw[&head_key(a)]).unwrap();
    head_a.content_hash = ContentHash::blake3(0, b"another");
    let mut cs_seq_a3 = rev_meta(raw, a, 3);
    cs_seq_a3.commit_seq = seq_of(raw, a, 2);
    let mut kind_a4 = rev_meta(raw, a, 4);
    kind_a4.kind = RevisionKind::Update;
    let mut extra_a6 = rev_meta(raw, a, 5);
    extra_a6.parent_rev = Some(RevNo(5));
    let create_a = seq_of(raw, a, 1);
    let delete_b = seq_of(raw, bu, 2);
    let mut cs_count = ChangeSetRecord::decode(&raw[&k(Key::ChangeSet { seq: delete_b })]).unwrap();
    cs_count.member_count += 1;
    let member_b = k(Key::CsMember { seq: delete_b, ordinal: 1 });
    let mut kind_member = CsMember::decode(&raw[&member_b]).unwrap();
    kind_member.kind = RevisionKind::Archive;
    let retention_seq = seq_of(raw, e, 4).0 + 1;
    let mut audit_c1 = rev_meta(raw, c, 1);
    audit_c1.payload_present = true;
    let mut none_d = UnitHead::decode(&raw[&head_key(d)]).unwrap();
    none_d.oldest_rev = RevNo(1);
    let mut huge_head = UnitHead::decode(&raw[&head_key(a)]).unwrap();
    huge_head.head_rev = RevNo(1 << 63);
    let mut gap_head = UnitHead::decode(&raw[&head_key(a)]).unwrap();
    gap_head.oldest_rev = RevNo(1);
    gap_head.head_rev = RevNo(u64::MAX - 1);
    let mut head_env = raw[&head_key(a)].clone();
    head_env[0] = 0;
    let mut head_env2 = raw[&head_key(a)].clone();
    head_env2[0] = 2;
    let mut bad_key = head_key(a);
    bad_key.truncate(5);
    let mut rev0 = k(Key::RevMeta { unit: a, rev: RevNo(1) });
    rev0[17..].fill(0);
    let newer = LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned }.encode().unwrap();
    let mut newer = newer;
    newer[5] = 2; // layer_version = 2
    vec![
        ("payload byte changed", vec![(pay(a, 2), Some(flipped))], "UNIT-28"),
        ("payload truncated", vec![(pay(a, 3), Some(raw[&pay(a, 3)][..4].to_vec()))], "UNIT-27"),
        ("head payload missing", vec![(pay(a, 5), None)], "UNIT-26"),
        ("payload of an audited revision", vec![(pay(c, 1), Some(vec![4; 10]))], "UNIT-26"),
        ("REV_META missing", vec![(k(Key::RevMeta { unit: a, rev: RevNo(2) }), None)], "UNIT-31"),
        ("head missing", vec![(head_key(bu), None)], "UNIT-30"),
        ("head disagrees", vec![(head_key(a), Some(head_a.encode().unwrap()))], "UNIT-31"),
        // A damaged head_rev must cost the verifier the records that exist,
        // not the range it claims.
        ("head_rev of 2^63", vec![(head_key(a), Some(huge_head.encode().unwrap()))], "UNIT-31"),
        ("huge gap below the head", vec![(head_key(a), Some(gap_head.encode().unwrap()))], "UNIT-31"),
        ("REV_META above the head", vec![put_meta(a, 6, &extra_a6)], "UNIT-31"),
        ("commit_seq out of order", vec![put_meta(a, 3, &cs_seq_a3)], "UNIT-32"),
        ("impossible transition", vec![put_meta(a, 4, &kind_a4)], "UNIT-32"),
        ("CHANGESET missing", vec![(k(Key::ChangeSet { seq: create_a }), None)], "UNIT-30"),
        ("CS_MEMBER missing", vec![(member_b.clone(), None)], "UNIT-33"),
        (
            "member_count wrong",
            vec![(k(Key::ChangeSet { seq: delete_b }), Some(cs_count.encode().unwrap()))],
            "UNIT-33",
        ),
        ("member kind wrong", vec![(member_b, Some(kind_member.encode().unwrap()))], "UNIT-33"),
        (
            "Retention member missing",
            vec![(k(Key::CsMember { seq: CommitSeq(retention_seq), ordinal: 0 }), None)],
            "UNIT-33",
        ),
        ("Audit kept a payload", vec![put_meta(c, 1, &audit_c1), (pay(c, 1), Some(vec![4; 10]))], "UNIT-34"),
        (
            "None kept a revision",
            vec![
                (k(Key::RevMeta { unit: d, rev: RevNo(1) }), Some(b.d_rev1.0.clone())),
                (pay(d, 1), Some(b.d_rev1.1.clone())),
                (head_key(d), Some(none_d.encode().unwrap())),
            ],
            "UNIT-34",
        ),
        ("mark missing", vec![(mark.clone(), None)], "UNIT-29"),
        ("mark of a newer layer", vec![(mark.clone(), Some(newer))], "UNIT-16"),
        ("mark damaged", vec![(mark, Some(vec![1, 0, 0, 0, 0, 0, 2]))], "UNIT-15"),
        (
            "unknown LAYER_META name, valid value",
            vec![(
                k(Key::LayerMeta { name: "zzz".into() }),
                Some(LayerMeta { layer_version: 1, history_policy: HistoryPolicy::None }.encode().unwrap()),
            )],
            "UNIT-29",
        ),
        ("unknown tag in a known range", vec![(vec![0x05, 1, 2], Some(b"v".to_vec()))], "UNIT-1"),
        ("tag of a newer layer", vec![(vec![0x77, 1], Some(b"v".to_vec()))], "UNIT-2"),
        ("key length", vec![(bad_key, Some(b"v".to_vec()))], "UNIT-3"),
        ("rev_no 0", vec![(rev0, Some(raw[&k(Key::RevMeta { unit: a, rev: RevNo(1) })].clone()))], "UNIT-5"),
        ("envelope 0", vec![(head_key(a), Some(head_env))], "UNIT-6"),
        ("envelope of a newer layer", vec![(head_key(a), Some(head_env2))], "UNIT-7"),
    ]
}

fn engine<E: Engine>(disk: &SimVfs) -> E {
    E::open(disk.as_vfs(), Path::new(DIR), &Options::small_for_tests()).expect("engine")
}

fn apply<E: Engine>(db: &E, patch: &Patch) {
    let mut b = WriteBatch::new();
    for (key, v) in patch {
        match v {
            Some(v) => b.put(key.clone(), v.clone()),
            None => b.delete(key.clone()),
        };
    }
    db.commit(&b).unwrap();
}

fn is_damage_error(e: &UnitError) -> bool {
    matches!(
        e,
        UnitError::Storage(kerno_core::Error::Corruption(_) | kerno_core::Error::Incompatible(_))
            | UnitError::IncompatibleLayer { .. }
            | UnitError::NotAUnitDatabase
    )
}

fn damage_detected<E: TxnEngine>() {
    let b = base::<E>(71);
    let a = b.ids.a;
    for (name, patch, rule) in damages(&b) {
        let disk = SimVfs::restore(&b.image);
        let raw_engine: E = engine(&disk);
        let before = raw_engine.snapshot().unwrap();
        apply(&raw_engine, &patch);
        let report = verify_engine(&raw_engine).unwrap();
        assert!(report.rules().contains(rule), "{name}: expected {rule}, found {:?}", report.findings);
        let class = report.findings.iter().find(|f| f.rule.id() == rule).unwrap().class();
        let want = if matches!(rule, "UNIT-2" | "UNIT-7" | "UNIT-16") {
            ErrorClass::IncompatibleLayer
        } else {
            ErrorClass::Corruption
        };
        assert_eq!(class, want, "{name}");
        let err = report.clone().into_result().unwrap_err();
        assert!(is_damage_error(&err), "{name}: {err}");
        // IU-1 sees the rewritten revisions.
        let rewritten = patch.iter().any(|(key, v)| {
            v.is_some() && matches!(key.first(), Some(0x20 | 0x21)) && before.get(key).unwrap().is_some()
        });
        let changed = verify_immutable(&before, &raw_engine.snapshot().unwrap()).unwrap();
        assert_eq!(!changed.is_empty(), rewritten, "{name}: {changed:?}");
        drop(before);
        raw_engine.close().unwrap();

        // The layer: a damaged mark refuses to open; otherwise the reads that
        // touch the damage fail with a damage class, never wrong data.
        match UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), &cfg()) {
            Err(e) => {
                assert!(name.starts_with("mark"), "{name}: open refused: {e}");
                assert!(is_damage_error(&e), "{name}: {e}");
                if rule == "UNIT-16" {
                    assert!(matches!(e, UnitError::IncompatibleLayer { found: 2, supported: 1 }), "{e}");
                }
                if name == "mark missing" {
                    assert!(matches!(e, UnitError::NotAUnitDatabase), "{e}");
                }
            }
            Ok(db) => {
                let r = db.verify_units().unwrap();
                assert_eq!(r.findings, report.findings, "{name}: same findings through the layer");
                let read = match name {
                    "payload byte changed" | "REV_META missing" => Some(db.get_revision(a, RevNo(2)).map(|_| ())),
                    "payload truncated" => Some(db.compare(a, RevNo(3), RevNo(5)).map(|_| ())),
                    "head payload missing" | "head disagrees" | "envelope 0" => Some(db.get_unit(a).map(|_| ())),
                    "envelope of a newer layer" => Some(db.get_unit(a).map(|_| ())),
                    _ => None,
                };
                if let Some(r) = read {
                    let e = r.expect_err(name);
                    assert!(is_damage_error(&e), "{name}: {e}");
                }
                // Writes over the damaged head refuse without effect.
                if matches!(name, "head payload missing" | "head disagrees" | "payload byte changed") {
                    let s = db.snapshot().unwrap().seq();
                    let mut cs = db.begin_changeset(meta()).unwrap();
                    if name != "payload byte changed" {
                        assert!(is_damage_error(&cs.archive(a, None).unwrap_err()), "{name}");
                    } else {
                        assert!(is_damage_error(&cs.restore(a, RevNo(2), None).unwrap_err()), "{name}");
                    }
                    cs.abort();
                    assert_eq!(db.snapshot().unwrap().seq(), s);
                }
                db.close().unwrap();
            }
        }
    }
}

// ------------------------------------------------------------ media damage

/// Bit flips and truncations of the files of a units store (§8.4, I-08).
fn media_damage<E: TxnEngine>() {
    let b = base::<E>(72);
    let disk = SimVfs::restore(&b.image);
    let db = UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), &cfg()).unwrap();
    let ids = [b.ids.a, b.ids.b, b.ids.c, b.ids.d, b.ids.e];
    let want: Vec<Option<Unit>> = ids.iter().map(|id| db.get_unit(*id).unwrap()).collect();
    let last = db.snapshot().unwrap().seq();
    db.close().unwrap();
    let (mut found, mut harmless, mut rolled_back) = (0, 0, 0);
    for seed in 0..60u64 {
        let mut rng = Rng::new(seed);
        let disk = SimVfs::restore(&b.image);
        let files: Vec<(PathBuf, u64)> = disk.files().into_iter().filter(|(_, len)| *len > 0).collect();
        let (path, len) = &files[rng.below(files.len() as u64) as usize];
        let offset = rng.below(*len);
        let what = if seed % 2 == 0 {
            let mask = 1u8 << rng.below(8);
            disk.corrupt_byte(path, offset, mask);
            format!("bit {mask:#04x} at {}+{offset}", path.display())
        } else {
            let f = disk.as_vfs().open(path, OpenMode::Existing).unwrap();
            f.set_len(offset).unwrap();
            f.sync_data().unwrap();
            format!("{} truncated to {offset}", path.display())
        };
        let ok_err = |e: &UnitError, stage: &str| {
            assert!(
                matches!(
                    e,
                    UnitError::Storage(
                        kerno_core::Error::Corruption(_) | kerno_core::Error::Io(_) | kerno_core::Error::Poisoned(_)
                    )
                ),
                "{what}: {stage}: {e}"
            );
        };
        let db = match UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), &cfg()) {
            Ok(db) => db,
            Err(e) => {
                ok_err(&e, "open");
                found += 1;
                continue;
            }
        };
        let seq = match db.snapshot() {
            Ok(s) => s.seq(),
            Err(e) => {
                ok_err(&e, "snapshot");
                found += 1;
                continue;
            }
        };
        if seq != last {
            // `KernoDb` never rolls back silently (I-08, the engine's damage
            // suites). The reference log engine may lose a truncated tail;
            // what is left must still be a whole prefix of change sets.
            assert_ne!(E::name(), KernoDb::name(), "{what}: silently rolled back from {last} to {seq}");
            assert!(seq < last, "{what}");
            let r = db.verify_units().unwrap_or_else(|e| panic!("{what}: verify_units: {e}"));
            assert!(r.is_clean(), "{what}: a recovered prefix verifies clean: {:?}", r.findings);
            rolled_back += 1;
            let _ = db.close();
            continue;
        }
        let mut detected = false;
        match db.verify_units() {
            Ok(r) if r.is_clean() => {}
            Ok(r) => {
                let e = r.into_result().unwrap_err();
                ok_err(&e, "verify_units");
                detected = true;
            }
            Err(e) => {
                ok_err(&e, "verify_units");
                detected = true;
            }
        }
        for (id, w) in ids.iter().zip(&want) {
            match db.get_unit(*id) {
                Ok(got) => assert_eq!(&got, w, "{what}: get_unit returned other data"),
                Err(e) => {
                    ok_err(&e, "get_unit");
                    detected = true;
                }
            }
        }
        if detected {
            found += 1;
        } else {
            harmless += 1;
        }
        let _ = db.close();
    }
    eprintln!("{}: media damage detected {found}, harmless {harmless}, prefix {rolled_back}", E::name());
}

// ------------------------------------------------------------ I-18

/// `verify_units` and the refused opens write nothing (I-18). On any engine
/// the store's keys and sequence stay the same; on `KernoDb` opened
/// read-only, the directory stays the same byte for byte.
fn verify_writes_nothing<E: TxnEngine>() {
    let b = base::<E>(73);
    let mark = k(Key::LayerMeta { name: "layer".into() });
    let mut newer = LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned }.encode().unwrap();
    newer[5] = 2;
    let variants: Vec<Patch> = vec![
        vec![],
        vec![(mark.clone(), None)],
        vec![(mark.clone(), Some(newer))],
        vec![(mark, Some(vec![0; 7]))],
        vec![(k(Key::RevPayload { unit: b.ids.a, rev: RevNo(1) }), Some(vec![0; 10]))],
    ];
    for patch in variants {
        let disk = SimVfs::restore(&b.image);
        if !patch.is_empty() {
            let e: E = engine(&disk);
            apply(&e, &patch);
            e.close().unwrap();
        }
        let e: E = engine(&disk);
        let before = raw_of(&e.snapshot().unwrap());
        let seq = e.snapshot().unwrap().seq();
        let r1 = verify_engine(&e).unwrap();
        let r2 = verify_engine(&e).unwrap();
        assert_eq!(r1, r2, "verification is deterministic");
        assert_eq!(r1.is_clean(), patch.is_empty());
        let opened = UnitDb::<E>::from_engine(e, &cfg());
        let e: E = match opened {
            Ok(db) => {
                assert_eq!(db.verify_units().unwrap(), r1);
                db.close().unwrap();
                engine(&disk)
            }
            Err(err) => {
                assert!(is_damage_error(&err), "{err}");
                engine(&disk)
            }
        };
        assert_eq!(e.snapshot().unwrap().seq(), seq, "nothing committed");
        assert!(raw_of(&e.snapshot().unwrap()) == before, "nothing changed");
        e.close().unwrap();
    }
}

/// I-18 on files: `KernoDb` opened read-only, verified and refused, leaves
/// the directory untouched (only `LOCK` may appear).
fn kerno_read_only_untouched() {
    let b = base::<KernoDb>(74);
    let mark = k(Key::LayerMeta { name: "layer".into() });
    let mut newer = LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned }.encode().unwrap();
    newer[5] = 2;
    for patch in [vec![], vec![(mark.clone(), None)], vec![(mark, Some(newer))]] {
        let disk = SimVfs::restore(&b.image);
        if !patch.is_empty() {
            let e: KernoDb = engine(&disk);
            apply(&e, &patch);
            e.close().unwrap();
        }
        let image = disk.dump();
        let ops = disk.mutating_ops();
        let ro = Config { access: AccessMode::ReadOnly, ..Config::from(&Options::small_for_tests()) };
        let e = KernoDb::open(disk.as_vfs(), Path::new(DIR), &ro).unwrap();
        let report = verify_engine(&e).unwrap();
        assert_eq!(report.is_clean(), patch.is_empty());
        match UnitDb::<KernoDb>::from_engine(e, &cfg()) {
            Ok(db) => {
                assert_eq!(db.verify_units().unwrap(), report);
                db.close().unwrap();
            }
            Err(err) => assert!(is_damage_error(&err), "{err}"),
        }
        assert_eq!(disk.mutating_ops(), ops, "verification did mutating operations");
        assert_dir_unchanged(&image, &disk.dump(), &["LOCK"]);
    }
}

#[test]
fn kerno_read_only_verify_untouched() {
    kerno_read_only_untouched();
}

#[test]
fn report_api() {
    let f = VerifyRule::Cross(CrossRule::Unit33);
    assert_eq!(f.id(), "UNIT-33");
    assert_eq!(VerifyRule::Cross(CrossRule::Unit29).id(), "UNIT-29");
    assert_eq!(VerifyRule::Cross(CrossRule::Unit35).id(), "UNIT-35");
    assert_eq!(VerifyRule::Structure(UnitRule::Unit2).class(), ErrorClass::IncompatibleLayer);
    assert_eq!(VerifyRule::Payload(PayloadRule::Unit28).class(), ErrorClass::Corruption);
}

macro_rules! engine_tests {
    ($m:ident, $e:ty) => {
        mod $m {
            use super::*;
            type E = $e;

            #[test]
            fn clean_scenario() {
                super::clean_scenario::<E>();
            }
            #[test]
            fn damage_detected() {
                super::damage_detected::<E>();
            }
            #[test]
            fn media_damage() {
                super::media_damage::<E>();
            }
            #[test]
            fn verify_writes_nothing() {
                super::verify_writes_nothing::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, Emulated<LogEngine>);
engine_tests!(kerno, KernoDb);
