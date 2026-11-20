//! I-29 for `kerno-units` (acta E9-03, E9A-02 as the model): the errors, the
//! `verify_units` reports and the observations of the layer carry no content.
//!
//! The detector has the shape of `kerno-storage/tests/redaction.rs`: the
//! content that enters the layer (unit type, metadata, payload, actor, source,
//! reason) carries sentinels, two of them plain text and two of them not valid
//! UTF-8, and every `Display` and `Debug` text the test collects is scanned
//! for them in every form a formatter could print them (text, hex, decimal
//! list, `\x` escapes) and, as the storage test does since E9-03, for
//! fragments of eight bytes or more of each. The test fails when a refusal
//! it relies on was not produced, so a scenario that stops reaching an error
//! path cannot pass. The scenario runs on `emulated_log` and on `kerno_db`.

mod harness;

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::ops::Bound;
use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::WriteBatch;
use kerno_core::contract::{Options, Snapshot};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::Key;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

/// A recognisable byte string.
struct Sentinel {
    name: &'static str,
    bytes: &'static [u8],
}

const TEXT_A: Sentinel = Sentinel { name: "text a", bytes: b"kerno-units-redaction-a-7f3a9c" };
const TEXT_B: Sentinel = Sentinel { name: "text b", bytes: b"kerno-units-redaction-b-c4e81b" };
const BIN_A: Sentinel =
    Sentinel { name: "binary a", bytes: &[0xde, 0xad, 0xbe, 0xef, 0x5e, 0x17, 0xc0, 0xde, 0x42, 0x99] };
const BIN_B: Sentinel =
    Sentinel { name: "binary b", bytes: &[0xfe, 0xed, 0xfa, 0xce, 0x0b, 0xad, 0xf0, 0x0d, 0x77, 0x31] };
const SENTINELS: [&Sentinel; 4] = [&TEXT_A, &TEXT_B, &BIN_A, &BIN_B];

/// Shortest run of a sentinel that counts as a fragment of it.
const FRAGMENT: usize = 8;

// ------------------------------------------------------------- detector

/// Lower-cased and without whitespace, so `{:#?}` of a byte list (one number
/// per line) and `{:X?}` compare like their compact forms.
fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// Every form in which `bytes` could appear in a message, normalised. The
/// text forms of bytes that are not UTF-8 would be replacement characters, which
/// say nothing about the bytes, so they are left out.
fn forms(bytes: &[u8]) -> Vec<(&'static str, String)> {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let escaped: String = bytes.iter().map(|b| format!("\\x{b:02x}")).collect();
    let ascii_escaped: String = bytes.iter().flat_map(|&b| std::ascii::escape_default(b)).map(char::from).collect();
    let strip = |s: String| s.trim_start_matches('[').trim_end_matches(']').to_string();
    let mut forms = vec![
        ("hex", hex),
        ("0x hex", bytes.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(",")),
        ("escaped", escaped),
        ("ascii-escaped", ascii_escaped),
        ("decimal list", strip(format!("{bytes:?}"))),
        ("hex list", strip(format!("{bytes:x?}"))),
    ];
    if let Ok(text) = std::str::from_utf8(bytes) {
        forms.push(("text", text.to_string()));
        forms.push(("text debug", format!("{text:?}").trim_matches('"').to_string()));
    }
    forms.into_iter().map(|(n, f)| (n, norm(&f))).collect()
}

/// Which sentinels, whole or in fragments, and forms `text` contains.
fn find_sentinels(text: &str) -> Vec<String> {
    let text = norm(text);
    let mut hits = Vec::new();
    for s in SENTINELS {
        for (form, needle) in forms(s.bytes) {
            if text.contains(&needle) {
                hits.push(format!("{} as {form}", s.name));
            }
        }
        for window in s.bytes.windows(FRAGMENT) {
            for (form, needle) in forms(window) {
                if text.contains(&needle) {
                    hits.push(format!("a fragment of {} as {form}", s.name));
                }
            }
        }
    }
    hits
}

/// What a scenario saw: every rendered text with where it came from.
#[derive(Default)]
struct Seen {
    texts: Vec<(String, String)>,
    errors: BTreeSet<&'static str>,
    findings: usize,
    /// Findings of a payload rule, from the damaged payloads: the path of
    /// `verify.rs` that formats a finding about stored content.
    payload_findings: usize,
    records: usize,
    /// Observation records that carry the shape text (`Diagnostic`), the
    /// text built from the filter: the one observation field made of a query.
    shape_texts: usize,
}

fn error_name(e: &UnitError) -> &'static str {
    match e {
        UnitError::Storage(kerno_core::Error::Corruption(_)) => "Storage(Corruption)",
        UnitError::Storage(_) => "Storage(other)",
        UnitError::NotFound { .. } => "NotFound",
        UnitError::Deleted { .. } => "Deleted",
        UnitError::Conflict { .. } => "Conflict",
        UnitError::RevisionNotFound { .. } => "RevisionNotFound",
        UnitError::Pruned { .. } => "Pruned",
        UnitError::PayloadTooLarge { .. } => "PayloadTooLarge",
        UnitError::ChangeSetTooLarge { .. } => "ChangeSetTooLarge",
        UnitError::InvalidArgument(_) => "InvalidArgument",
        UnitError::UnitHasRelations { .. } => "UnitHasRelations",
        UnitError::IndexNotReady { .. } => "IndexNotReady",
        _ => "other",
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

    fn error(&mut self, label: &str, e: &UnitError) {
        self.errors.insert(error_name(e));
        self.text(label, e.to_string());
        self.debug(label, e);
    }

    /// Records the error. An `Ok` is what the caller asked for, which holds
    /// its own content on purpose; reports go through [`Seen::report`].
    fn outcome<T>(&mut self, label: &str, r: &Result<T, UnitError>) {
        if let Err(e) = r {
            self.error(label, e);
        }
    }

    /// Records the error, or the `Debug` of a report or a summary.
    fn report<T: Debug>(&mut self, label: &str, r: &Result<T, UnitError>) {
        match r {
            Ok(v) => self.debug(label, v),
            Err(e) => self.error(label, e),
        }
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

fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::stepping(T0, 1_000_000)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

/// A sentinel followed by a number, as the keys and values of an application do.
fn tagged(s: &Sentinel, i: u32) -> Vec<u8> {
    let mut v = s.bytes.to_vec();
    v.extend(format!("-{i:03}").bytes());
    v
}

fn meta() -> ChangeMeta {
    ChangeMeta {
        reason: Some(String::from_utf8(TEXT_A.bytes.to_vec()).unwrap()),
        actor: Some(BIN_A.bytes.to_vec()),
        source: Some(TEXT_B.bytes.to_vec()),
        correlation: Some(BIN_B.bytes.to_vec()),
        purpose: Purpose::Application,
    }
}

fn new_unit(i: u32) -> NewUnit {
    NewUnit {
        unit_type: String::from_utf8(tagged(&TEXT_A, i)).unwrap(),
        schema_version: 1,
        metadata: tagged(if i.is_multiple_of(2) { &BIN_A } else { &BIN_B }, i),
        payload: tagged(if i.is_multiple_of(2) { &TEXT_B } else { &BIN_B }, i),
    }
}

/// Two units with three revisions each and relations between them.
fn plant<E: Target>(db: &UnitDb<E>) -> [UnitId; 2] {
    let mut cs = db.begin_changeset(meta()).unwrap();
    let a = cs.create(new_unit(0)).unwrap();
    let b = cs.create(new_unit(1)).unwrap();
    cs.create_relation(a, b, "refers", RelationProvenance::Imported { source: tagged(&BIN_A, 1) }).unwrap();
    cs.create_relation(b, a, "refers", RelationProvenance::Algorithmic { origin: tagged(&TEXT_B, 2) }).unwrap();
    cs.commit().unwrap();
    for i in 2..4u32 {
        let mut cs = db.begin_changeset(meta()).unwrap();
        for id in [a, b] {
            let upd =
                UnitUpdate { payload: tagged(&BIN_B, i), schema_version: None, metadata: Some(tagged(&TEXT_A, i)) };
            cs.update(id, upd, None).unwrap();
        }
        cs.commit().unwrap();
    }
    [a, b]
}

/// A damage: the records of the store and a unit in, the records to rewrite
/// (`None` removes one) out.
type Damage = fn(&[(Vec<u8>, Vec<u8>)], UnitId) -> Vec<(Vec<u8>, Option<Vec<u8>>)>;

fn raw_rows<S: Snapshot>(snap: &S) -> Vec<(Vec<u8>, Vec<u8>)> {
    snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap()
}

/// What the layer says about a store that is damaged in one record: the
/// verifier's findings and the errors of the reads that meet the damage.
fn damaged_reads<E: Target>(
    seen: &mut Seen,
    label: &str,
    disk: &SimVfs,
    ids: [UnitId; 2],
    patch: impl FnOnce(&[(Vec<u8>, Vec<u8>)]) -> Vec<(Vec<u8>, Option<Vec<u8>>)>,
) {
    let engine = E::open_engine(disk.as_vfs(), &config().engine).expect("reopen the engine");
    let rows = raw_rows(&engine.snapshot().unwrap());
    let mut batch = WriteBatch::new();
    for (k, v) in patch(&rows) {
        match v {
            Some(v) => drop(batch.put(k, v)),
            None => drop(batch.delete(k)),
        }
    }
    engine.commit(&batch).expect("raw commit");

    match verify_engine(&engine) {
        Ok(report) => {
            seen.findings += report.findings.len();
            seen.debug(&format!("{label}: report"), &report);
            seen.text(label, format!("{}", report.counts.units));
            for f in &report.findings {
                seen.payload_findings += usize::from(matches!(f.rule, VerifyRule::Payload(_)));
                seen.text(&format!("{label}: finding"), f.to_string());
                seen.debug(&format!("{label}: finding"), f);
                seen.error(&format!("{label}: finding as error"), &f.to_error());
            }
            seen.outcome(&format!("{label}: into_result"), &report.clone().into_result().map(|_| ()));
        }
        Err(e) => seen.error(&format!("{label}: verify_engine"), &e),
    }

    let db = match UnitDb::from_engine(engine, &config()) {
        Ok(db) => db,
        Err(e) => return seen.error(&format!("{label}: open"), &e),
    };
    seen.report(&format!("{label}: verify_units"), &db.verify_units());
    for id in ids {
        seen.outcome(&format!("{label}: get_unit"), &db.get_unit(id));
        for rev in 1..=4u64 {
            seen.outcome(&format!("{label}: get_revision {rev}"), &db.get_revision(id, RevNo(rev)));
        }
        seen.outcome(&format!("{label}: history"), &db.history(id, HistoryQuery::latest(10)));
    }
    seen.outcome(&format!("{label}: query"), &db.snapshot().and_then(|s| s.query(&UnitFilter::default())));
}

/// Every refusal and every kind of damage, then the observations.
fn scenario<E: Target>() -> Seen {
    let mut seen = Seen::default();
    let disk = SimVfs::new(1);
    let db = open_units::<E>(disk.as_vfs(), &config()).expect("open");
    let ids = plant(&db);

    // Refusals before any write, all naming content.
    let mut long_type = tagged(&TEXT_A, 9);
    long_type.extend([b'x'; 600]);
    let long_type = String::from_utf8(long_type).unwrap();
    let too_big = NewUnit { payload: [BIN_A.bytes, &vec![0u8; 2 << 20]].concat(), ..new_unit(5) };
    let other = NewUnit { unit_type: long_type, ..new_unit(6) };
    let empty_type = NewUnit { unit_type: String::new(), ..new_unit(7) };
    let long_meta = ChangeMeta { reason: Some("r".repeat(2000)), ..meta() };
    let long_actor = ChangeMeta { actor: Some([BIN_A.bytes, &[7u8; 5000]].concat()), ..meta() };
    seen.outcome("refusal: payload too large", &db.create_unit(too_big, meta()).map(|_| ()));
    seen.outcome("refusal: long type", &db.create_unit(other, meta()).map(|_| ()));
    seen.outcome("refusal: empty type", &db.create_unit(empty_type, meta()).map(|_| ()));
    seen.outcome("refusal: long reason", &db.begin_changeset(long_meta).map(|_| ()));
    seen.outcome("refusal: long actor", &db.begin_changeset(long_actor).map(|_| ()));
    seen.outcome("not found", &db.get_revision(UnitId([0x5e; 16]), RevNo(1)));
    seen.outcome("revision not found", &db.get_revision(ids[0], RevNo(99)));

    // Conflict, and a unit with live relations.
    let mut cs = db.begin_changeset(meta()).unwrap();
    let upd = UnitUpdate { payload: tagged(&TEXT_B, 8), schema_version: None, metadata: Some(tagged(&BIN_A, 8)) };
    seen.outcome("conflict", &cs.update(ids[0], upd, Some(RevNo(1))));
    seen.outcome("unit has relations", &cs.archive(ids[0], None));
    seen.outcome("delete with relations", &cs.delete(ids[1], None));
    cs.abort();

    // An index that is not ready, then history pruned by retention.
    let snap = db.snapshot().unwrap();
    seen.outcome("index not ready", &snap.query(&UnitFilter::default()).map(|o| (o.path, o.fallback)));
    drop(snap);
    let report = db.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default());
    seen.report("retention report", &report);
    seen.outcome("pruned", &db.get_revision(ids[0], RevNo(1)));

    // Observations, at every level.
    for level in [
        ObservationLevel::Off,
        ObservationLevel::Aggregate,
        ObservationLevel::Fingerprint,
        ObservationLevel::Diagnostic,
    ] {
        db.observer().set_level(level).unwrap();
        db.observer().set_key(FingerprintKey::from_bytes([7; 32]));
        db.observer().clear();
        let filter =
            UnitFilter { unit_type: Some(String::from_utf8(tagged(&TEXT_A, 0)).unwrap()), schema_version: Some(1) };
        let snap = db.snapshot().unwrap();
        seen.report("observed query", &snap.query(&filter).map(|o| (o.source, o.path, o.fallback)));
        seen.outcome(
            "observed relations",
            &snap
                .query_relations(&RelationQuery {
                    unit: ids[0],
                    direction: Direction::Out,
                    relation_type: Some("refers".into()),
                })
                .map(|o| (o.path, o.fallback)),
        );
        seen.debug(&format!("stats {level:?}"), &db.observer().stats());
        for r in db.observer().records() {
            seen.records += 1;
            seen.shape_texts += usize::from(r.fingerprint_text.as_deref().is_some_and(|t| t.starts_with("units[")));
            seen.debug(&format!("record {level:?}"), &r);
        }
        if level >= ObservationLevel::Fingerprint {
            let key = FingerprintKey::from_bytes([7; 32]);
            let plan = snap.explain_filter(&key, &filter);
            seen.report(&format!("explain {level:?}"), &plan.map(|p| format!("{p:?}")));
        }
    }
    db.close().unwrap();

    // Damage, one record at a time, on its own copy of a closed store that
    // retention has not touched (three revisions of each unit).
    let fresh = SimVfs::new(2);
    let db = open_units::<E>(fresh.as_vfs(), &config()).expect("open");
    let ids = plant(&db);
    db.close().unwrap();
    let closed = fresh.dump();
    let kinds: [(&str, Damage); 6] = [
        ("flipped payload", |rows, a| {
            let k = Key::RevPayload { unit: a, rev: RevNo(3) }.encode().unwrap();
            let mut v = rows.iter().find(|(rk, _)| *rk == k).unwrap().1.clone();
            *v.last_mut().unwrap() ^= 1;
            vec![(k, Some(v))]
        }),
        ("cut payload", |rows, a| {
            let k = Key::RevPayload { unit: a, rev: RevNo(2) }.encode().unwrap();
            let v = rows.iter().find(|(rk, _)| *rk == k).unwrap().1[..4].to_vec();
            vec![(k, Some(v))]
        }),
        ("missing payload", |_, a| vec![(Key::RevPayload { unit: a, rev: RevNo(3) }.encode().unwrap(), None)]),
        ("missing revision", |_, a| vec![(Key::RevMeta { unit: a, rev: RevNo(2) }.encode().unwrap(), None)]),
        ("head with a byte more", |rows, a| {
            let k = Key::UnitHead { unit: a }.encode().unwrap();
            let mut v = rows.iter().find(|(rk, _)| *rk == k).unwrap().1.clone();
            v.push(0);
            vec![(k, Some(v))]
        }),
        ("head cut", |rows, a| {
            let k = Key::UnitHead { unit: a }.encode().unwrap();
            let mut v = rows.iter().find(|(rk, _)| *rk == k).unwrap().1.clone();
            v.truncate(v.len() - 1);
            vec![(k, Some(v))]
        }),
    ];
    for (name, patch) in kinds {
        let copy = SimVfs::restore(&closed);
        damaged_reads::<E>(&mut seen, name, &copy, ids, |rows| patch(rows, ids[0]));
    }
    seen
}

fn check_case<E: Target>() {
    let seen = scenario::<E>();
    let needed = [
        "Storage(Corruption)",
        "NotFound",
        "Conflict",
        "RevisionNotFound",
        "PayloadTooLarge",
        "InvalidArgument",
        "UnitHasRelations",
        "Pruned",
    ];
    let missing: Vec<_> = needed.iter().filter(|v| !seen.errors.contains(*v)).collect();
    assert!(missing.is_empty(), "{}: errors never produced: {missing:?}; saw {:?}", E::NAME, seen.errors);
    assert!(seen.findings > 0, "{}: verify_units reported no finding, so the report path was not exercised", E::NAME);
    assert!(seen.records > 0, "{}: no observation was recorded", E::NAME);
    // The report path that formats a finding about stored content ran: the
    // flipped payload and the cut one are two such findings at least.
    assert!(
        seen.payload_findings >= 2,
        "{}: {} payload findings; the payload path of verify did not run",
        E::NAME,
        seen.payload_findings
    );
    // The shape text of the filtered query was recorded at `Diagnostic`.
    assert!(seen.shape_texts >= 1, "{}: no observation carried a shape text", E::NAME);
    let leaks = seen.leaks();
    assert!(
        leaks.is_empty(),
        "{}: {} of {} texts carry a sentinel (I-29); first ones:\n{}",
        E::NAME,
        leaks.len(),
        seen.texts.len(),
        leaks.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// The detector itself: every form of every sentinel, and a fragment of it,
/// is found; text without one is not. Without this, a broken needle would
/// read as a pass.
#[test]
fn detector_finds_every_form_and_fragment_of_every_sentinel() {
    for s in SENTINELS {
        let owned = s.bytes.to_vec();
        let part = &s.bytes[2..2 + FRAGMENT];
        let part_owned = part.to_vec();
        let mut renderings = vec![
            format!("{owned:?}"),
            format!("{owned:#?}"),
            format!("{owned:x?}"),
            format!("{owned:X?}"),
            s.bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            s.bytes.iter().map(|b| format!("\\x{b:02x}")).collect::<String>(),
            s.bytes.iter().flat_map(|&b| std::ascii::escape_default(b)).map(char::from).collect::<String>(),
            format!("unit {:?} is missing", tagged(s, 7)),
            // Fragments.
            format!("{part_owned:?}"),
            format!("{part_owned:x?}"),
            part.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            part.iter().map(|b| format!("\\x{b:02x}")).collect::<String>(),
        ];
        if let Ok(text) = std::str::from_utf8(s.bytes) {
            renderings.push(format!("{text:?}"));
            renderings.push(text[2..2 + FRAGMENT].to_string());
        }
        for r in renderings {
            let wrapped = format!("corruption detected: revision 3 of unit 0001: {r} (rule UNIT-07)");
            assert!(!find_sentinels(&wrapped).is_empty(), "{}: not found in {wrapped:?}", s.name);
        }
    }
    let clean = "corruption detected: REV_PAYLOAD of unit 00000000000000000000000000000001 rev 4: \
                 content hash differs at offset 4096, crc 0x1f2e3d4c, 55 entries (UNIT-07)";
    assert!(find_sentinels(clean).is_empty(), "{:?}", find_sentinels(clean));
    // Seven bytes of a sentinel are under the fragment length and are not a finding.
    let short: String = String::from_utf8_lossy(&TEXT_A.bytes[..FRAGMENT - 1]).into_owned();
    assert!(find_sentinels(&short).is_empty());
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            #[test]
            fn units_leak_no_content_in_errors_reports_or_observations() {
                super::check_case::<$engine>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
