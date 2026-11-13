//! E6-04: sentinels for the result cache (etapa6-acta.md §3, ADR-0016 G7,
//! ADR-0015, I-37).
//!
//! `cache.rs` (E6-01) already plants the sentinels of `sentinels.rs` and
//! drives the *store* through its transitions. These tests close the two
//! things that file leaves open:
//!
//! * `no_literal_in_anything_the_cache_path_reports` — the same sentinels, but
//!   through the real planner with the cache on, at each observation level:
//!   the explanation of each read (`source`, `path`, `fallback`), the
//!   explanation of the plan, the cache's records and counters, the
//!   observer's records, and every error the path can give (a refused entry,
//!   a refused bound, a read that fails on a damaged head). No sentinel
//!   appears, and every fingerprint reported is that of a shape with other
//!   literals. The answers handed to the caller do hold the sentinels (the
//!   control), so the search could have found them.
//! * `every_g7_transition_leaves_its_record` — one case per leaf of
//!   [`Transition`] (every cause, dependency, reason and switch), each on a
//!   fresh store, each required to leave exactly its record with the
//!   optimization and planner versions and, where it concerns an entry, the
//!   fingerprint and the sequence of the entry. `label` matches without a
//!   wildcard: a new variant does not compile until it is named there and in
//!   [`ALL_LABELS`], where the case that walks it must be added.
//!
//! The sentinel constants repeat those of `sentinels.rs` and `cache.rs`: each
//! test target compiles alone, and the values only have to be strings no layer
//! writes by itself.

mod harness;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::Key;
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

fn assert_clean(what: &str, text: &str) {
    for (kind, needles) in sentinels() {
        for n in needles {
            assert!(!text.contains(n.as_str()), "{kind} sentinel {n:?} leaked in {what}:\n{text}");
        }
    }
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

/// A write whose content holds sentinels: it moves `commit_seq` and leaves
/// the index stale.
fn write_sentinels<E: Target>(db: &UnitDb<E>, unit: UnitId) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let update = UnitUpdate { payload: S_PAYLOAD.to_vec(), schema_version: Some(S_VERSION), metadata: None };
    cs.update(unit, update, None).unwrap();
    cs.commit().unwrap();
}

/// Fingerprints of the same shapes with other literals: one that hashed a
/// literal differs from all of these.
fn literal_free_shapes(key: &FingerprintKey, unit: UnitId) -> Vec<Fingerprint> {
    let units = filters().into_iter().map(|f| {
        NormalizedQuery::of_filter(&UnitFilter {
            unit_type: f.unit_type.map(|_| "other".into()),
            schema_version: f.schema_version.map(|_| 1),
        })
    });
    let relations = [Direction::Out, Direction::In].into_iter().map(|direction| {
        NormalizedQuery::of_relations(&RelationQuery { unit, direction, relation_type: Some("other".into()) })
    });
    units.chain(relations).map(|q| q.fingerprint(key)).collect()
}

fn assert_shape_only(reported: impl IntoIterator<Item = Fingerprint>, shapes: &[Fingerprint], what: &str) {
    for fp in reported {
        assert!(shapes.contains(&fp), "fingerprint {fp} in {what} depends on a literal");
    }
}

/// Truncates the head of `unit` in the engine, so reading it is corruption.
fn damage_head<E: Target>(db: &UnitDb<E>, unit: UnitId) {
    let head = Key::UnitHead { unit }.encode().unwrap();
    let good = db.raw_get(&head).unwrap().unwrap();
    db.raw_commit(vec![(head, Some(good[..good.len() - 1].to_vec()))]).expect("raw commit");
}

fn evidence() -> Evidence {
    Evidence { observations: 1, window: Duration::from_secs(1) }
}

fn deps_of<E: Target>(snap: &UnitSnapshot<E>, with_index: bool) -> Dependencies {
    Dependencies::current(snap.seq(), with_index.then(|| snap.index_status().unwrap()))
}

/// A cache entry for `query` holding `result`, valid for `snap` in every
/// dependency.
fn entry_for<E: Target>(snap: &UnitSnapshot<E>, fp: Fingerprint, query: CachedQuery, rows: CachedResult) -> CacheEntry {
    let with_index = matches!(query, CachedQuery::Units(_));
    CacheEntry::new(fp, query, deps_of(snap, with_index), rows, evidence(), 1_000).unwrap()
}

// ---------------------------------------------------------------------------
// 1. Nothing the cache path reports holds a literal, at each level.

/// What a pass says, as text, and what the caller was handed.
#[derive(Default)]
struct Out {
    text: String,
    answers: String,
}

impl Out {
    fn say(&mut self, what: &str, body: String) {
        self.text.push_str(&format!("[{what}] {body}\n"));
    }

    fn units<E: Target>(&mut self, snap: &UnitSnapshot<E>, f: &UnitFilter) -> Option<PlanPath> {
        match snap.query(f) {
            Ok(o) => {
                self.answers.push_str(&format!("{:?}", o.units));
                self.say("units", format!("{:?} {:?} {:?}", o.source, o.path, o.fallback));
                if o.path == PlanPath::ResultCache {
                    assert!(matches!(o.source, QuerySource::Cache { .. }), "the explanation names the learned state");
                }
                Some(o.path)
            }
            Err(e) => {
                self.say("query error", format!("{e} | {e:?}"));
                None
            }
        }
    }

    fn relations<E: Target>(&mut self, snap: &UnitSnapshot<E>, q: &RelationQuery) -> Option<PlanPath> {
        match snap.query_relations(q) {
            Ok(o) => {
                self.answers.push_str(&format!("{:?}", o.relations));
                self.say("relations", format!("{:?} {:?}", o.path, o.fallback));
                Some(o.path)
            }
            Err(e) => {
                self.say("relations error", format!("{e} | {e:?}"));
                None
            }
        }
    }
}

/// One pass over every level. `damaged` says the head of a unit has been
/// truncated: reads then end in an error, which is output under search too.
fn pass<E: Target>(db: &UnitDb<E>, ids: [UnitId; 2], damaged: bool) -> Vec<(ObservationLevel, String)> {
    let observer_key = FingerprintKey::from_bytes([7; 32]);
    let cache_key = FingerprintKey::from_bytes([9; 32]);
    let cache = db.result_cache();
    let mut outputs = Vec::new();

    for level in LEVELS {
        db.observer().clear();
        db.observer().set_level(level).unwrap();
        db.observer().set_key(observer_key.clone());
        if !damaged {
            cache.set_enabled(false);
            cache.set_key(cache_key.clone()); // drops the learned state
            cache.set_enabled(true);
            cache.clear();
        }
        let mut out = Out::default();

        let snap = db.snapshot().unwrap();
        for f in filters() {
            match snap.explain_filter(&cache_key, &f) {
                Ok(plan) => out.say("explain", format!("{plan:?} {}", plan.fingerprint())),
                Err(e) => out.say("explain error", format!("{e} | {e:?}")),
            }
            let first = out.units(&snap, &f);
            let second = out.units(&snap, &f);
            if !damaged {
                assert_ne!(first, Some(PlanPath::ResultCache), "the first read has nothing to serve");
                // An entry needs the observer's evidence: below `Fingerprint` there is none.
                assert_eq!(
                    second == Some(PlanPath::ResultCache),
                    level >= ObservationLevel::Fingerprint,
                    "{level:?}: the second read at one sequence is served only with evidence"
                );
            }
        }
        let rq = relation_query(ids[0]);
        let first = out.relations(&snap, &rq);
        let second = out.relations(&snap, &rq);
        if !damaged {
            assert_ne!(first, Some(PlanPath::ResultCache));
            assert_eq!(
                second == Some(PlanPath::ResultCache),
                level >= ObservationLevel::Fingerprint,
                "{level:?}: relations are served from the cache too, with evidence"
            );
        }

        if !damaged {
            // Control: what the caller is handed does hold the sentinels.
            for (kind, needles) in sentinels().iter().filter(|(k, _)| *k != "provenance origin") {
                let held = needles.iter().any(|n| out.answers.contains(n.as_str()));
                assert!(held, "control: the answers hold the {kind}");
            }

            // Another query of the same shape: a miss of its own.
            out.units(&snap, &UnitFilter { unit_type: Some("other".into()), schema_version: None });

            // A forged entry that fails the checks: a discrepancy.
            let f = filters()[0].clone();
            let fp = cache.fingerprint_of(&CachedQuery::Units(f.clone())).unwrap();
            let mut rows = snap.query(&f).unwrap().units;
            rows.reverse();
            cache.create(entry_for(&snap, fp, CachedQuery::Units(f.clone()), CachedResult::Units(rows))).unwrap();
            cache.activate(&fp).unwrap();
            out.units(&snap, &f);

            // The per-query switch.
            cache.disable_query(fp);
            out.units(&snap, &f);
            cache.enable_query(fp);
            out.units(&snap, &f);

            // A write with sentinels in it invalidates through the sequence.
            write_sentinels(db, ids[0]);
            let later = db.snapshot().unwrap();
            out.units(&later, &f);
            out.units(&later, &f);
            db.rebuild_index().unwrap();

            // The errors of the store: a refusal of each kind.
            let any = || CachedQuery::Units(UnitFilter::default());
            let empty = || CachedResult::Units(Vec::new());
            let now = deps_of(&later, true);
            let refused = [
                CacheEntry::new(fp, any(), deps_of(&later, false), empty(), evidence(), 1_000),
                CacheEntry::new(fp, any(), now, CachedResult::Relations(Vec::new()), evidence(), 1_000),
                CacheEntry::new(fp, any(), now, empty(), Evidence { observations: 0, window: Duration::ZERO }, 1_000),
            ];
            for r in refused {
                let e = r.unwrap_err();
                out.say("entry error", format!("{e} | {e:?}"));
            }
            let unknown = cache.fingerprint_of(&CachedQuery::Units(UnitFilter::default())).unwrap();
            let e = cache.activate(&unknown).unwrap_err();
            out.say("activate error", format!("{e} | {e:?}"));
            let held = CachedResult::Units(later.query(&f).unwrap().units);
            cache.set_config(CacheConfig { max_bytes: 1, ..CacheConfig::default() }).unwrap();
            let e = cache.create(entry_for(&later, fp, CachedQuery::Units(f.clone()), held)).unwrap_err();
            out.say("create error", format!("{e} | {e:?}"));
            cache.set_config(CacheConfig::default()).unwrap();
            let e = cache.set_config(CacheConfig { max_entries: 0, ..CacheConfig::default() }).unwrap_err();
            out.say("config error", format!("{e} | {e:?}"));
            cache.set_enabled(false);
            let e = cache.create(entry_for(&later, fp, CachedQuery::Units(f), empty())).unwrap_err();
            out.say("disabled error", format!("{e} | {e:?}"));
            cache.set_enabled(true);
        }

        // What the cache and the observer report.
        let mut reported_by_cache = Vec::new();
        for r in cache.records() {
            reported_by_cache.extend(r.fingerprint);
            out.say("cache record", format!("{r:#?}"));
        }
        out.say("cache stats", format!("{:?}", cache.stats()));
        let observed = db.observer().records();
        for r in &observed {
            out.say("observer record", format!("{r:#?}"));
        }
        out.say("observer stats", format!("{:?}", db.observer().stats()));

        assert_shape_only(reported_by_cache, &literal_free_shapes(&cache_key, ids[1]), "the cache's records");
        let shapes = literal_free_shapes(&observer_key, ids[1]);
        assert_shape_only(observed.iter().map(|r| r.fingerprint), &shapes, "the observer's records");

        if !damaged {
            let kinds: BTreeSet<String> = cache.records().iter().map(|r| label(&r.transition)).collect();
            for wanted in [
                "Created",
                "Activated",
                "Used",
                "Fallback/Discrepancy",
                "Deleted/Discrepancy",
                "Fallback/Disabled",
                "Disabled/Query",
                "Enabled/Query",
            ] {
                assert!(kinds.contains(wanted), "level {level:?}: no {wanted} record in {kinds:?}");
            }
            // These come from entries the planner created itself, which needs the
            // observer's evidence: below `Fingerprint` it creates none.
            if level >= ObservationLevel::Fingerprint {
                for wanted in ["Missed/OtherQuery", "Invalidated/IndexState", "Fallback/Invalidated"] {
                    assert!(kinds.contains(wanted), "level {level:?}: no {wanted} record in {kinds:?}");
                }
            }
            if matches!(level, ObservationLevel::Fingerprint | ObservationLevel::Diagnostic) {
                assert!(
                    observed.iter().any(|r| r.path == Some(PlanPath::ResultCache)),
                    "level {level:?}: the observer records an execution served by the cache"
                );
            }
        }
        outputs.push((level, out.text));
    }
    outputs
}

fn no_literal_in_anything_the_cache_path_reports<E: Target>() {
    let db = open::<E>();
    let ids = plant(&db);
    db.rebuild_index().unwrap();

    let mut outputs = pass(&db, ids, false);
    // Damage the head of the second unit: the reference path now fails, and
    // with the cache still holding entries from before, the errors come from
    // a read that the cache could not serve.
    damage_head(&db, ids[1]);
    outputs.extend(pass(&db, ids, true));

    assert!(
        outputs.iter().any(|(_, o)| o.contains("[query error]")),
        "the damaged head must surface as a query error that this test searched"
    );
    assert!(outputs.iter().any(|(_, o)| o.contains("[entry error]")), "the refusals of the store were searched");
    for (level, text) in &outputs {
        assert_clean(&format!("the output at level {level:?}"), text);
    }
}

// ---------------------------------------------------------------------------
// 2. Every G7 transition leaves its record.

/// Every leaf of [`Transition`], named. No wildcard: adding a variant, a cause,
/// a reason, a switch or a dependency does not compile until it is named here.
fn label(t: &Transition) -> String {
    match t {
        Transition::Created { .. } => "Created".into(),
        Transition::Activated => "Activated".into(),
        Transition::Used => "Used".into(),
        Transition::Missed { cause } => format!(
            "Missed/{}",
            match cause {
                MissCause::NoEntry => "NoEntry",
                MissCause::OtherQuery => "OtherQuery",
                MissCause::NotActive => "NotActive",
                MissCause::Expired => "Expired",
            }
        ),
        Transition::Fallback { cause } => format!(
            "Fallback/{}",
            match cause {
                CacheFallbackCause::PreconditionFailed => "PreconditionFailed",
                CacheFallbackCause::Invalidated => "Invalidated",
                CacheFallbackCause::Disabled => "Disabled",
                CacheFallbackCause::Discrepancy => "Discrepancy",
            }
        ),
        Transition::Invalidated { dependency } => format!(
            "Invalidated/{}",
            match dependency {
                Dependency::SchemaVersion => "SchemaVersion",
                Dependency::PlannerVersion => "PlannerVersion",
                Dependency::IndexState => "IndexState",
                Dependency::IndexWatermark => "IndexWatermark",
                Dependency::CommitSeq => "CommitSeq",
            }
        ),
        Transition::Enabled { switch } => format!("Enabled/{}", switch_name(*switch)),
        Transition::Disabled { switch } => format!("Disabled/{}", switch_name(*switch)),
        Transition::Deleted { reason } => format!(
            "Deleted/{}",
            match reason {
                DeleteReason::Requested => "Requested",
                DeleteReason::Replaced => "Replaced",
                DeleteReason::EvictedBySize => "EvictedBySize",
                DeleteReason::EvictedByAge => "EvictedByAge",
                DeleteReason::Disabled => "Disabled",
                DeleteReason::Discrepancy => "Discrepancy",
            }
        ),
        Transition::Cleared { .. } => "Cleared".into(),
        Transition::KeyReplaced { .. } => "KeyReplaced".into(),
    }
}

fn switch_name(s: Switch) -> &'static str {
    match s {
        Switch::Global => "Global",
        Switch::Query => "Query",
    }
}

/// The leaves, by hand. The walk below must produce a record for each, and
/// every record it produces must be one of these.
const ALL_LABELS: [&str; 27] = [
    "Created",
    "Activated",
    "Used",
    "Missed/NoEntry",
    "Missed/OtherQuery",
    "Missed/NotActive",
    "Missed/Expired",
    "Fallback/PreconditionFailed",
    "Fallback/Invalidated",
    "Fallback/Disabled",
    "Fallback/Discrepancy",
    "Invalidated/SchemaVersion",
    "Invalidated/PlannerVersion",
    "Invalidated/IndexState",
    "Invalidated/IndexWatermark",
    "Invalidated/CommitSeq",
    "Enabled/Global",
    "Enabled/Query",
    "Disabled/Global",
    "Disabled/Query",
    "Deleted/Requested",
    "Deleted/Replaced",
    "Deleted/EvictedBySize",
    "Deleted/EvictedByAge",
    "Deleted/Disabled",
    "Deleted/Discrepancy",
    "Cleared",
];

/// What a case needs: a store on a clock the case can move, an entry for a
/// real read of the sentinel data, and the dependencies it was read under.
struct Rig {
    cache: ResultCache,
    clock: Arc<FixedClock>,
    query: CachedQuery,
    other_query: CachedQuery,
    fp: Fingerprint,
    second: (Fingerprint, CachedQuery, CachedResult),
    entry: CacheEntry,
    deps: Dependencies,
}

impl Rig {
    fn on(&self) {
        self.cache.set_enabled(true);
    }
    fn fill(&self) {
        self.on();
        self.cache.create(self.entry.clone()).unwrap();
        self.cache.activate(&self.fp).unwrap();
    }
    fn lookup_with(&self, now: Dependencies) -> Lookup {
        self.cache.lookup(&self.fp, &self.query, &now)
    }
    fn far_future(&self) {
        self.clock.set(Timestamp(T0.0 + 2 * 3600 * 1_000_000));
    }
}

type Case = (&'static str, Option<Switch>, fn(&Rig));

fn invalidated_by(dependency: Dependency) -> fn(&Rig) {
    // One function pointer per dependency: the closures capture nothing.
    match dependency {
        Dependency::SchemaVersion => |r| {
            let now = Dependencies { schema_version: r.deps.schema_version + 1, ..r.deps };
            r.fill();
            assert!(matches!(r.lookup_with(now), Lookup::Invalidated(Dependency::SchemaVersion)));
        },
        Dependency::PlannerVersion => |r| {
            let now = Dependencies { planner_version: r.deps.planner_version + 1, ..r.deps };
            r.fill();
            assert!(matches!(r.lookup_with(now), Lookup::Invalidated(Dependency::PlannerVersion)));
        },
        Dependency::IndexState => |r| {
            let failed = IndexStatus { state: IndexState::Failed, watermark: None };
            let now = Dependencies { index: Some(failed), ..r.deps };
            r.fill();
            assert!(matches!(r.lookup_with(now), Lookup::Invalidated(Dependency::IndexState)));
        },
        Dependency::IndexWatermark => |r| {
            let moved = IndexStatus { state: IndexState::Ready, watermark: Some(CommitSeq(r.deps.commit_seq.0 + 9)) };
            let now = Dependencies { index: Some(moved), ..r.deps };
            r.fill();
            assert!(matches!(r.lookup_with(now), Lookup::Invalidated(Dependency::IndexWatermark)));
        },
        Dependency::CommitSeq => |r| {
            let now = Dependencies { commit_seq: CommitSeq(r.deps.commit_seq.0 + 1), ..r.deps };
            r.fill();
            assert!(matches!(r.lookup_with(now), Lookup::Invalidated(Dependency::CommitSeq)));
        },
    }
}

fn cases() -> Vec<Case> {
    vec![
        ("Created", None, |r| {
            r.on();
            r.cache.create(r.entry.clone()).unwrap();
        }),
        ("Activated", None, |r| r.fill()),
        ("Used", None, |r| {
            r.fill();
            assert!(matches!(r.lookup_with(r.deps), Lookup::Hit(_)));
        }),
        ("Missed/NoEntry", None, |r| {
            r.on();
            assert!(matches!(r.lookup_with(r.deps), Lookup::Miss));
        }),
        ("Missed/OtherQuery", None, |r| {
            r.fill();
            assert!(matches!(r.cache.lookup(&r.fp, &r.other_query, &r.deps), Lookup::Miss));
        }),
        ("Missed/NotActive", None, |r| {
            r.on();
            r.cache.create(r.entry.clone()).unwrap();
            assert!(matches!(r.lookup_with(r.deps), Lookup::Inactive));
        }),
        ("Missed/Expired", None, |r| {
            r.fill();
            r.far_future();
            assert!(matches!(r.lookup_with(r.deps), Lookup::Miss));
        }),
        ("Fallback/PreconditionFailed", None, |r| {
            r.fill();
            r.cache.fallback(&r.fp, CacheFallbackCause::PreconditionFailed);
        }),
        ("Fallback/Invalidated", None, |r| {
            r.fill();
            r.cache.fallback(&r.fp, CacheFallbackCause::Invalidated);
        }),
        ("Fallback/Disabled", None, |r| {
            r.fill();
            r.cache.fallback(&r.fp, CacheFallbackCause::Disabled);
        }),
        ("Fallback/Discrepancy", None, |r| {
            r.fill();
            r.cache.fallback(&r.fp, CacheFallbackCause::Discrepancy);
        }),
        ("Invalidated/SchemaVersion", None, invalidated_by(Dependency::SchemaVersion)),
        ("Invalidated/PlannerVersion", None, invalidated_by(Dependency::PlannerVersion)),
        ("Invalidated/IndexState", None, invalidated_by(Dependency::IndexState)),
        ("Invalidated/IndexWatermark", None, invalidated_by(Dependency::IndexWatermark)),
        ("Invalidated/CommitSeq", None, invalidated_by(Dependency::CommitSeq)),
        ("Enabled/Global", Some(Switch::Global), |r| r.on()),
        ("Enabled/Query", Some(Switch::Query), |r| {
            r.on();
            r.cache.disable_query(r.fp);
            r.cache.enable_query(r.fp);
        }),
        ("Disabled/Global", Some(Switch::Global), |r| {
            r.on();
            r.cache.set_enabled(false);
        }),
        ("Disabled/Query", Some(Switch::Query), |r| {
            r.on();
            r.cache.disable_query(r.fp);
        }),
        ("Deleted/Requested", None, |r| {
            r.fill();
            assert!(r.cache.delete(&r.fp));
        }),
        ("Deleted/Replaced", None, |r| {
            r.fill();
            r.cache.create(r.entry.clone()).unwrap();
        }),
        ("Deleted/EvictedBySize", None, |r| {
            r.fill();
            r.cache.set_config(CacheConfig { max_entries: 1, ..CacheConfig::default() }).unwrap();
            let (fp, query, rows) = r.second.clone();
            let entry = CacheEntry::new(fp, query, r.deps, rows, evidence(), 1_000).unwrap();
            r.cache.create(entry).unwrap();
        }),
        ("Deleted/EvictedByAge", None, |r| {
            r.fill();
            r.far_future();
            r.cache.set_config(CacheConfig::default()).unwrap();
        }),
        ("Deleted/Disabled", None, |r| {
            r.fill();
            r.cache.set_enabled(false);
        }),
        ("Deleted/Discrepancy", None, |r| {
            r.fill();
            r.cache.fallback(&r.fp, CacheFallbackCause::Discrepancy);
        }),
        ("Cleared", None, |r| {
            r.fill();
            r.cache.clear();
        }),
    ]
}

fn rig<E: Target>(db: &UnitDb<E>) -> Rig {
    let snap = db.snapshot().unwrap();
    let cache = ResultCache::new();
    let clock = Arc::new(FixedClock::stepping(T0, 1));
    cache.set_clock(clock.clone());
    cache.set_key(FingerprintKey::from_bytes([9; 32]));
    let (f, g) = (filters()[0].clone(), filters()[1].clone());
    let query = CachedQuery::Units(f.clone());
    let other_query = CachedQuery::Units(UnitFilter { unit_type: Some("other".into()), schema_version: None });
    let fp = cache.fingerprint_of(&query).unwrap();
    let rows = CachedResult::Units(snap.query(&f).unwrap().units);
    let second_query = CachedQuery::Units(g.clone());
    let second = (
        cache.fingerprint_of(&second_query).unwrap(),
        second_query,
        CachedResult::Units(snap.query(&g).unwrap().units),
    );
    let entry = entry_for(&snap, fp, query.clone(), rows);
    let deps = deps_of(&snap, true);
    Rig { cache, clock, query, other_query, fp, second, entry, deps }
}

fn every_g7_transition_leaves_its_record<E: Target>() {
    let db = open::<E>();
    plant(&db);
    db.rebuild_index().unwrap();

    let leaves: BTreeSet<&str> = ALL_LABELS.iter().copied().collect();
    let cases = cases();
    let walked: BTreeSet<&str> = cases.iter().map(|c| c.0).collect();
    assert_eq!(walked, leaves, "the walk has one case for each leaf of the transition log, and no other");

    for (wanted, switch, run) in cases {
        let r = rig(&db);
        run(&r);
        let records = r.cache.records();
        let found: Vec<&TransitionRecord> = records.iter().filter(|rec| label(&rec.transition) == wanted).collect();
        assert!(!found.is_empty(), "{wanted}: no record in {records:#?}");
        let rec = found[0];

        // The versions of G7, on every record.
        assert_eq!(rec.optimization_version, OPTIMIZATION_VERSION, "{wanted}");
        assert_eq!(rec.planner_version, PLANNER_VERSION, "{wanted}");
        // Who it is about: a query's fingerprint, or nobody (the global switch, a clear).
        let global = switch == Some(Switch::Global) || wanted == "Cleared";
        match rec.fingerprint {
            None => assert!(global, "{wanted}: a record about a query has its fingerprint"),
            Some(fp) => {
                assert!(!global, "{wanted}: a global record has no fingerprint");
                assert!(fp == r.fp || fp == r.second.0, "{wanted}: the fingerprint is that of the query");
            }
        }
        // What it carries about the entry: its sequence, and the evidence of its creation.
        if ["Created", "Activated", "Used", "Invalidated"].iter().any(|k| wanted.starts_with(k)) {
            assert_eq!(rec.commit_seq, Some(r.deps.commit_seq), "{wanted}: the sequence of the entry");
        }
        if wanted == "Created" {
            assert_eq!(
                rec.transition,
                Transition::Created { evidence: r.entry.evidence(), cost_ns: r.entry.cost_ns() }
            );
        }
        if wanted == "Cleared" {
            assert_eq!(rec.transition, Transition::Cleared { entries: 1 });
        }

        // The record, its counters and the store's errors hold no literal.
        assert_clean(wanted, &format!("{records:#?} {:?}", r.cache.stats()));
        for e in [
            CacheError::Disabled,
            CacheError::KindMismatch,
            CacheError::NoEvidence,
            CacheError::TooLarge,
            CacheError::NoEntry,
        ] {
            assert_clean("a cache error", &format!("{e} | {e:?}"));
        }
        assert_clean("an index status error", &format!("{}", CacheError::IndexStatus { kind: QueryKind::Units }));
    }
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn no_literal_in_anything_the_cache_path_reports() {
                super::no_literal_in_anything_the_cache_path_reports::<E>();
            }
            #[test]
            fn every_g7_transition_leaves_its_record() {
                super::every_g7_transition_leaves_its_record::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
