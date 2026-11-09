//! E6-01: the result cache's dependencies and transition log against a real
//! database (etapa6-acta.md §3, ADR-0016 G4 and G7, ADR-0015, I-35, I-37).
//!
//! The store's own behaviour (bounds, switches, one case per dependency) is
//! tested next to it in `src/cache.rs`. These tests need the layer:
//!
//! * `a_commit_invalidates_the_cached_result_of_an_earlier_snapshot` — the
//!   dependencies come from a real snapshot (`seq`, `index_status`), and a
//!   real commit, a real index rebuild and a snapshot at the same sequence
//!   each do what G4 says: the first invalidates, the last serves.
//! * `no_filter_literal_in_any_transition_record_at_any_level` — I-37 for the
//!   transition log. The sentinels of `sentinels.rs` are planted in the data
//!   and in the filters; the cache is filled with the real answers (which hold
//!   them) and driven through every transition at each observation level; no
//!   sentinel appears in any record, counter or error the cache reports.

mod harness;

use std::sync::Arc;
use std::time::Duration;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

const S_TYPE: &str = "SNT-TYPE-7f3a91c2";
const S_PAYLOAD: &[u8] = b"SNT-PAYLOAD-5be0d447";
const S_METADATA: &[u8] = b"SNT-METADATA-c9a1e6f0";
const S_REL_TYPE: &str = "SNT-RELTYPE-28d4b7a5";
const S_SOURCE: &[u8] = b"SNT-SOURCE-61f3a0cc";
const S_ORIGIN: &[u8] = b"SNT-ORIGIN-e4079b3d";
/// The value of the schema version filter, as a number a text search can find.
const S_VERSION: u32 = 4_071_337_791;

const LEVELS: [ObservationLevel; 4] =
    [ObservationLevel::Off, ObservationLevel::Aggregate, ObservationLevel::Fingerprint, ObservationLevel::Diagnostic];

/// Each sentinel in every form a `Debug` or `Display` of it could take: the
/// text, the decimal list of a byte slice and hex.
fn forms(bytes: &[u8]) -> Vec<String> {
    let list = bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    vec![String::from_utf8(bytes.to_vec()).unwrap(), list, hex]
}

fn sentinels() -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("unit type", forms(S_TYPE.as_bytes())),
        ("payload", forms(S_PAYLOAD)),
        ("metadata", forms(S_METADATA)),
        ("relation type", forms(S_REL_TYPE.as_bytes())),
        ("provenance source", forms(S_SOURCE)),
        ("provenance origin", forms(S_ORIGIN)),
        ("schema version", vec![S_VERSION.to_string()]),
    ]
}

fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: Target>() -> UnitDb<E> {
    open_units::<E>(SimVfs::new(1).as_vfs(), &config()).expect("open")
}

fn new_unit(t: &str, schema: u32) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: schema, metadata: S_METADATA.to_vec(), payload: S_PAYLOAD.to_vec() }
}

/// Two units that carry every sentinel, related by three relations.
fn plant<E: Target>(db: &UnitDb<E>) -> [UnitId; 2] {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let a = cs.create(new_unit(S_TYPE, S_VERSION)).unwrap();
    let b = cs.create(new_unit(S_TYPE, 1)).unwrap();
    cs.create_relation(a, b, S_REL_TYPE, RelationProvenance::Imported { source: S_SOURCE.to_vec() }).unwrap();
    cs.create_relation(b, a, S_REL_TYPE, RelationProvenance::Algorithmic { origin: S_ORIGIN.to_vec() }).unwrap();
    cs.create_relation(
        a,
        b,
        S_REL_TYPE,
        RelationProvenance::Inferred { origin: S_ORIGIN.to_vec(), confidence_permille: 500 },
    )
    .unwrap();
    cs.commit().unwrap();
    [a, b]
}

fn filters() -> Vec<UnitFilter> {
    vec![
        UnitFilter { unit_type: Some(S_TYPE.into()), schema_version: None },
        UnitFilter { unit_type: None, schema_version: Some(S_VERSION) },
        UnitFilter { unit_type: Some(S_TYPE.into()), schema_version: Some(S_VERSION) },
    ]
}

fn relation_query(unit: UnitId) -> RelationQuery {
    RelationQuery { unit, direction: Direction::Out, relation_type: Some(S_REL_TYPE.into()) }
}

/// The dependencies in force for a read of `snap` that saw `index`.
fn deps_of<E: Target>(snap: &UnitSnapshot<E>, with_index: bool) -> Dependencies {
    let index = with_index.then(|| snap.index_status().unwrap());
    Dependencies::current(snap.seq(), index)
}

fn evidence_from(db_stats: &ObservationStats) -> Evidence {
    Evidence { observations: db_stats.observed.max(1), window: Duration::from_secs(60) }
}

fn on_cache() -> ResultCache {
    let cache = ResultCache::new();
    cache.set_clock(Arc::new(FixedClock::stepping(T0, 1)));
    cache.set_enabled(true);
    cache
}

fn a_commit_invalidates_the_cached_result_of_an_earlier_snapshot<E: Target>() {
    let db = open::<E>();
    let ids = plant(&db);
    db.rebuild_index().unwrap();
    let key = FingerprintKey::from_bytes([7; 32]);
    let cache = on_cache();
    let ev = Evidence { observations: 2, window: Duration::from_secs(1) };

    // Fill the cache from a real read, with the dependencies of its snapshot.
    let snap = db.snapshot().unwrap();
    let filter = &filters()[0];
    let uq = CachedQuery::Units(filter.clone());
    let fp = *snap.explain_filter(&key, filter).unwrap().fingerprint();
    let outcome = snap.query(filter).unwrap();
    let before = deps_of(&snap, true);
    cache
        .create(CacheEntry::new(fp, uq.clone(), before, CachedResult::Units(outcome.units.clone()), ev).unwrap())
        .unwrap();
    cache.activate(&fp).unwrap();

    // Another snapshot at the same sequence has the same dependencies: served.
    let same = db.snapshot().unwrap();
    assert_eq!(deps_of(&same, true), before);
    match cache.lookup(&fp, &uq, &deps_of(&same, true)) {
        Lookup::Hit(r) => assert_eq!(*r, CachedResult::Units(outcome.units.clone())),
        other => panic!("expected a hit, got {other:?}"),
    }

    // A commit moves the sequence and makes the index stale: not served, and
    // the first dependency that differs is named.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create(new_unit("other", 1)).unwrap();
    let commit = cs.commit().unwrap();
    let after = db.snapshot().unwrap();
    assert_eq!(after.seq(), commit.commit_seq);
    let now = deps_of(&after, true);
    assert_ne!(now.commit_seq, before.commit_seq);
    let expected = before.first_change(&now).expect("a commit changes a dependency");
    match cache.lookup(&fp, &uq, &now) {
        Lookup::Invalidated(d) => assert_eq!(d, expected),
        other => panic!("expected Invalidated, got {other:?}"),
    }
    assert_eq!(cache.stats().entries, 0);

    // Refill at the new snapshot, then rebuild the index. A rebuild is itself
    // a commit (the index generation is written through a change set), so the
    // sequence moves and the entry of the old snapshot is not served.
    let outcome = after.query(filter).unwrap();
    cache
        .create(CacheEntry::new(fp, uq.clone(), now, CachedResult::Units(outcome.units.clone()), ev).unwrap())
        .unwrap();
    cache.activate(&fp).unwrap();
    db.rebuild_index().unwrap();
    let rebuilt = db.snapshot().unwrap();
    assert!(rebuilt.seq() > after.seq(), "a rebuild commits");
    let rebuilt_deps = deps_of(&rebuilt, true);
    let expected = now.first_change(&rebuilt_deps).expect("a rebuild changes a dependency");
    match cache.lookup(&fp, &uq, &rebuilt_deps) {
        Lookup::Invalidated(d) => assert_eq!(d, expected),
        other => panic!("expected Invalidated, got {other:?}"),
    }
    cache
        .create(CacheEntry::new(fp, uq.clone(), rebuilt_deps, CachedResult::Units(outcome.units), ev).unwrap())
        .unwrap();
    cache.activate(&fp).unwrap();
    assert!(matches!(cache.lookup(&fp, &uq, &rebuilt_deps), Lookup::Hit(_)));

    // A relation query records no index status and its entry is keyed apart.
    let q = relation_query(ids[0]);
    let rq = CachedQuery::Relations(q.clone());
    let rfp = *rebuilt.explain_relations(&key, &q).unwrap().fingerprint();
    assert_ne!(rfp, fp);
    let rows = rebuilt.query_relations(&q).unwrap().relations;
    let rdeps = deps_of(&rebuilt, false);
    cache.create(CacheEntry::new(rfp, rq.clone(), rdeps, CachedResult::Relations(rows), ev).unwrap()).unwrap();
    cache.activate(&rfp).unwrap();
    assert!(matches!(cache.lookup(&rfp, &rq, &rdeps), Lookup::Hit(_)));
    let moved = Dependencies { commit_seq: CommitSeq(rdeps.commit_seq.0 + 1), ..rdeps };
    assert!(matches!(cache.lookup(&rfp, &rq, &moved), Lookup::Invalidated(Dependency::CommitSeq)));
}

/// Drives every transition for one fingerprint and returns everything the
/// cache says about it as text.
fn drive(cache: &ResultCache, fp: Fingerprint, entry: &CacheEntry, deps: Dependencies, tail: &mut String) {
    let mut say = |what: &str, text: String| tail.push_str(&format!("[{what}] {text}\n"));
    cache.create(entry.clone()).unwrap();
    cache.activate(&fp).unwrap();
    say("lookup", format!("{:?}", cache.lookup(&fp, entry.query(), &deps).clone_without_rows()));
    for cause in [
        CacheFallbackCause::PreconditionFailed,
        CacheFallbackCause::Invalidated,
        CacheFallbackCause::Disabled,
        CacheFallbackCause::Discrepancy,
    ] {
        cache.fallback(&fp, cause);
    }
    // Invalidated by every dependency in turn (each drops the entry: refill).
    for dependency in Dependency::ALL {
        let now = match dependency {
            Dependency::SchemaVersion => Dependencies { schema_version: deps.schema_version + 1, ..deps },
            Dependency::PlannerVersion => Dependencies { planner_version: deps.planner_version + 1, ..deps },
            Dependency::IndexState => {
                Dependencies { index: deps.index.map(|_| flip_state()).or(Some(flip_state())), ..deps }
            }
            Dependency::IndexWatermark => Dependencies {
                index: Some(IndexStatus {
                    state: IndexState::Ready,
                    watermark: Some(CommitSeq(deps.commit_seq.0 + 9)),
                }),
                ..deps
            },
            Dependency::CommitSeq => Dependencies { commit_seq: CommitSeq(deps.commit_seq.0 + 1), ..deps },
        };
        cache.create(entry.clone()).unwrap();
        cache.activate(&fp).unwrap();
        say("invalidated", format!("{:?}", cache.lookup(&fp, entry.query(), &now).clone_without_rows()));
    }
    cache.create(entry.clone()).unwrap();
    cache.create(entry.clone()).unwrap(); // replaces
    assert!(cache.delete(&fp));
    cache.disable_query(fp);
    say("refused", format!("{} | {:?}", CacheError::Disabled, cache.create(entry.clone())));
    cache.enable_query(fp);
    cache.create(entry.clone()).unwrap();
    cache.set_enabled(false);
    cache.set_enabled(true);
    cache.set_config(CacheConfig { max_entries: 1, ..CacheConfig::default() }).unwrap();
    cache.set_config(CacheConfig::default()).unwrap();
}

fn flip_state() -> IndexStatus {
    IndexStatus { state: IndexState::Failed, watermark: None }
}

/// A lookup's `Debug` without the rows: the rows are user content the cache
/// holds and returns to its caller, not something it reports.
trait WithoutRows {
    fn clone_without_rows(&self) -> String;
}

impl WithoutRows for Lookup {
    fn clone_without_rows(&self) -> String {
        match self {
            Lookup::Hit(_) => "Hit(..)".into(),
            other => format!("{other:?}"),
        }
    }
}

fn no_filter_literal_in_any_transition_record_at_any_level<E: Target>() {
    let db = open::<E>();
    let ids = plant(&db);
    db.rebuild_index().unwrap();
    let key = FingerprintKey::from_bytes([7; 32]);
    let wanted = sentinels();
    let mut outputs = Vec::new();

    for level in LEVELS {
        db.observer().clear();
        db.observer().set_level(level).unwrap();
        db.observer().set_key(key.clone());
        let cache = on_cache();
        let snap = db.snapshot().unwrap();
        let mut text = String::new();
        let mut reported = Vec::new();

        let mut jobs: Vec<(Fingerprint, CachedQuery, CachedResult, Dependencies)> = Vec::new();
        for f in filters() {
            let fp = *snap.explain_filter(&key, &f).unwrap().fingerprint();
            let rows = snap.query(&f).unwrap().units;
            jobs.push((fp, CachedQuery::Units(f.clone()), CachedResult::Units(rows), deps_of(&snap, true)));
        }
        let q = relation_query(ids[0]);
        let fp = *snap.explain_relations(&key, &q).unwrap().fingerprint();
        let rows = snap.query_relations(&q).unwrap().relations;
        jobs.push((fp, CachedQuery::Relations(q.clone()), CachedResult::Relations(rows), deps_of(&snap, false)));

        // Control: the rows the cache is about to hold do contain the sentinels,
        // so a search of the records for them is a search that could succeed.
        let held = format!("{:?}", jobs.iter().map(|j| &j.2).collect::<Vec<_>>());
        for (what, needles) in wanted.iter().filter(|(w, _)| !["provenance origin", "provenance source"].contains(w)) {
            assert!(needles.iter().any(|n| held.contains(n.as_str())), "control: the cached rows hold the {what}");
        }

        let ev = evidence_from(&db.observer().stats());
        for (fp, query, result, deps) in jobs {
            let entry = CacheEntry::new(fp, query, deps, result, ev).unwrap();
            reported.push(fp);
            drive(&cache, fp, &entry, deps, &mut text);
        }
        // A refusal of each kind: the errors are output under search too.
        let units_fp = reported[0];
        let rel_deps = deps_of(&snap, false);
        let any_units = || CachedQuery::Units(UnitFilter::default());
        let bad = [
            CacheEntry::new(units_fp, any_units(), rel_deps, CachedResult::Units(Vec::new()), ev),
            CacheEntry::new(units_fp, any_units(), deps_of(&snap, true), CachedResult::Relations(Vec::new()), ev),
            CacheEntry::new(
                units_fp,
                CachedQuery::Relations(relation_query(ids[0])),
                deps_of(&snap, true),
                CachedResult::Units(Vec::new()),
                ev,
            ),
            CacheEntry::new(
                units_fp,
                any_units(),
                deps_of(&snap, true),
                CachedResult::Units(Vec::new()),
                Evidence { observations: 0, window: Duration::ZERO },
            ),
        ];
        for r in bad {
            let e = r.unwrap_err();
            text.push_str(&format!("[error] {e} | {e:?}\n"));
        }

        let records = cache.records();
        for r in &records {
            text.push_str(&format!("[record] {r:#?}\n"));
            if let Some(fp) = r.fingerprint {
                text.push_str(&format!("[fingerprint] {fp} {fp:?}\n"));
            }
        }
        text.push_str(&format!("[stats] {:?}\n", cache.stats()));
        cache.clear();
        text.push_str(&format!("[after clear] {:?} {:?}\n", cache.records(), cache.stats()));

        // Every fingerprint in the log is that of a shape with other literals.
        let other = |f: UnitFilter| NormalizedQuery::of_filter(&f).fingerprint(&key);
        let mut shapes = vec![
            other(UnitFilter { unit_type: Some("x".into()), schema_version: None }),
            other(UnitFilter { unit_type: None, schema_version: Some(1) }),
            other(UnitFilter { unit_type: Some("x".into()), schema_version: Some(1) }),
        ];
        shapes.push(
            NormalizedQuery::of_relations(&RelationQuery {
                unit: ids[1],
                direction: Direction::Out,
                relation_type: Some("x".into()),
            })
            .fingerprint(&key),
        );
        for r in &records {
            if let Some(fp) = r.fingerprint {
                assert!(shapes.contains(&fp), "fingerprint {fp} at level {level:?} depends on a literal");
            }
        }

        // Not vacuous: every kind of transition is in the output.
        for kind in ["Created", "Activated", "Used", "Fallback", "Invalidated", "Enabled", "Disabled", "Deleted"] {
            assert!(
                records.iter().any(|r| format!("{:?}", r.transition).starts_with(kind)),
                "no {kind} record at level {level:?}"
            );
        }
        outputs.push((level, text));
    }

    for (level, text) in &outputs {
        for (what, needles) in &wanted {
            for n in needles {
                assert!(!text.contains(n.as_str()), "{what} sentinel {n:?} leaked at level {level:?}:\n{text}");
            }
        }
    }
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn a_commit_invalidates_the_cached_result_of_an_earlier_snapshot() {
                super::a_commit_invalidates_the_cached_result_of_an_earlier_snapshot::<E>();
            }
            #[test]
            fn no_filter_literal_in_any_transition_record_at_any_level() {
                super::no_filter_literal_in_any_transition_record_at_any_level::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
