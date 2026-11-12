//! E8-06: consolidation with provenance (etapa8-acta.md §5.2 and §5.3 point 5,
//! D12, I-31, I-32, I-43).
//!
//! Every test runs on both engines. Summaries are in memory only (module
//! documentation of `consolidation`); the summarizer is the deterministic one
//! the crate ships, plus a test double that proves the reserved parameter.

mod harness;

use std::collections::BTreeMap;
use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::{Options, TxnEngine};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::{CrashKind, OpKind, SimVfs};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open_db<E: Target>(disk: &SimVfs) -> UnitDb<E> {
    open_units::<E>(disk.as_vfs(), &cfg()).expect("open")
}

fn trunc() -> TruncatingSummarizer {
    TruncatingSummarizer::new(4, 1)
}

fn fresh<E: Target>(disk: &SimVfs) -> DerivedDb<E> {
    open_summaries(open_db::<E>(disk), Vec::new(), &trunc()).unwrap().0
}

fn make<E: TxnEngine>(db: &DerivedDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn edit<E: TxnEngine>(db: &DerivedDb<E>, id: UnitId, payload: &[u8]) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

fn delete<E: TxnEngine>(db: &DerivedDb<E>, id: UnitId) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(id, None).unwrap();
    cs.commit().unwrap();
}

/// A summarizer that tries to use the reserved parameter.
struct Sneaky;

impl Summarizer for Sneaky {
    fn method(&self) -> &str {
        "sneaky"
    }
    fn version(&self) -> u32 {
        1
    }
    fn params(&self) -> BTreeMap<String, String> {
        BTreeMap::from([(KIND_PARAM.to_owned(), "other".to_owned())])
    }
    fn summarize(&self, _: &[SourceInput]) -> Option<Vec<u8>> {
        Some(Vec::new())
    }
}

/// The shipped summarizer under another method name: same version and parameters.
struct Renamed(TruncatingSummarizer);

impl Summarizer for Renamed {
    fn method(&self) -> &str {
        "renamed"
    }
    fn version(&self) -> u32 {
        self.0.version()
    }
    fn params(&self) -> BTreeMap<String, String> {
        self.0.params()
    }
    fn summarize(&self, s: &[SourceInput]) -> Option<Vec<u8>> {
        self.0.summarize(s)
    }
}

/// A summarizer that cannot produce a text.
struct Mute;

impl Summarizer for Mute {
    fn method(&self) -> &str {
        "mute"
    }
    fn version(&self) -> u32 {
        1
    }
    fn summarize(&self, _: &[SourceInput]) -> Option<Vec<u8>> {
        None
    }
}

fn plain(_: &DerivedDescriptor, _: &[SourceInput]) -> Option<Vec<u8>> {
    Some(b"x".to_vec())
}

// ------------------------------------------------------------ contract

fn a_summary_records_its_exact_sources_and_a_full_descriptor<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(1));
    let (a, b) = (make(&db, b"alphabet"), make(&db, b"bravo"));
    edit(&db, b, b"bravo2");
    let id = db.build_summary([b, a, a], &trunc()).unwrap();
    assert_eq!(db.state(&id), Some(DerivedState::Ready));
    assert!(db.is_summary(&id));
    assert_eq!(db.summary(&id).as_deref(), Some(&b"alph\nbrav"[..]));

    let d = db.descriptor(&id).unwrap();
    let want = [RevisionId { unit: a, rev: RevNo(1) }, RevisionId { unit: b, rev: RevNo(2) }];
    assert_eq!(d.sources().collect::<Vec<_>>(), want, "exact revisions, as a set");
    assert_eq!(d.method(), TruncatingSummarizer::METHOD);
    assert_eq!(d.version(), 1);
    assert_eq!(d.params().get("max_bytes").map(String::as_str), Some("4"));
    assert_eq!(d.params().get(KIND_PARAM).map(String::as_str), Some("summary"));
    let inputs: Vec<SourceInput> = [a, b].iter().map(|u| source_of(&db, *u)).collect();
    assert_eq!(d.input_hash(), &input_hash(&inputs));
}

fn source_of<E: TxnEngine>(db: &DerivedDb<E>, unit: UnitId) -> SourceInput {
    let u = db.get_unit(unit).unwrap().unwrap();
    SourceInput { id: RevisionId { unit, rev: u.head }, content_hash: u.content_hash, payload: u.payload.unwrap() }
}

fn a_summary_with_no_sources_is_an_error<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(2));
    let _ = make(&db, b"a");
    assert!(matches!(db.build_summary(Vec::<UnitId>::new(), &trunc()), Err(ConsolidationError::NoSources)));
    // A source that does not exist is no source either.
    let missing = UnitId([9; 16]);
    assert!(matches!(
        db.build_summary([missing], &trunc()),
        Err(ConsolidationError::Derived(DerivedStaleError::MissingSource(m))) if m == missing
    ));
}

fn the_kind_parameter_is_reserved_and_separates_summaries<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(3));
    let a = make(&db, b"abcdefgh");
    assert!(matches!(db.build_summary([a], &Sneaky), Err(ConsolidationError::ReservedParam)));
    assert!(matches!(db.supersede_summary(&DerivedId([0; 32]), &Sneaky), Err(ConsolidationError::Derived(_))));

    // A derived datum that is not a summary is neither served nor changed by
    // the summary methods, and has another identity over the same unit.
    let other = db.build([a], "test.truncate", 1, BTreeMap::new(), &plain).unwrap();
    let s = db.build_summary([a], &trunc()).unwrap();
    assert_ne!(other, s);
    assert!(!db.is_summary(&other));
    assert_eq!(db.summary(&other), None);
    assert_eq!(db.value(&other).as_deref(), Some(&b"x"[..]));
    assert!(matches!(db.rebuild_summary(&other, &trunc()), Err(ConsolidationError::NotASummary(i)) if i == other));
    assert!(matches!(db.supersede_summary(&other, &trunc()), Err(ConsolidationError::NotASummary(i)) if i == other));
    assert_eq!(db.state(&other), Some(DerivedState::Ready));
    assert!(!db.is_summary(&DerivedId([0; 32])));
}

fn a_summarizer_that_gives_nothing_leaves_a_failed_summary<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(4));
    let a = make(&db, b"a");
    match db.build_summary([a], &Mute) {
        Err(ConsolidationError::Derived(DerivedStaleError::ComputeFailed(id))) => {
            assert_eq!(db.state(&id), Some(DerivedState::Failed));
            assert_eq!(db.summary(&id), None);
        }
        other => panic!("unexpected {other:?}"),
    }
}

// ------------------------------------------------------------ the original

/// Everything a reader can learn about the sources, head and history.
fn heads<E: TxnEngine>(db: &DerivedDb<E>, units: &[UnitId]) -> Vec<Option<Unit>> {
    units.iter().map(|u| db.get_unit(*u).unwrap()).collect()
}

fn the_original_stays_readable_through_every_summary_operation<E: Target>() {
    let disk = SimVfs::new(5);
    let db = fresh::<E>(&disk);
    let (a, b) = (make(&db, b"alphabet"), make(&db, b"bravo"));
    let before = heads(&db, &[a, b]);

    let s1 = db.build_summary([a, b], &trunc()).unwrap();
    assert_eq!(heads(&db, &[a, b]), before, "building");
    let s2 = db.supersede_summary(&s1, &TruncatingSummarizer::new(2, 1)).unwrap();
    assert_eq!(db.state(&s1), Some(DerivedState::Superseded));
    assert_eq!(db.summary(&s1), None);
    assert_eq!(db.summary(&s2).as_deref(), Some(&b"al\nbr"[..]));
    assert_eq!(heads(&db, &[a, b]), before, "superseding");

    edit(&db, a, b"alpha2");
    let after_edit = heads(&db, &[a, b]);
    assert_eq!(db.state(&s2), Some(DerivedState::Stale));
    let s3 = db.rebuild_summary(&s2, &TruncatingSummarizer::new(2, 1)).unwrap();
    assert_eq!(db.state(&s3), Some(DerivedState::Ready));
    assert_eq!(heads(&db, &[a, b]), after_edit, "rebuilding");

    // A summary is never what a read of its source returns.
    let ua = db.get_unit(a).unwrap().unwrap();
    assert_eq!(ua.payload.as_deref(), Some(&b"alpha2"[..]));
    assert_ne!(Some(ua.payload.unwrap()), db.summary(&s3));

    // Losing a summary (a source deleted) leaves the other source alone.
    delete(&db, a);
    assert_eq!(db.state(&s3), None);
    let ub = db.get_unit(b).unwrap().unwrap();
    assert_eq!(ub.payload.as_deref(), Some(&b"bravo"[..]));
    // The deleted source is still a unit with its revisions on disk.
    let ua = db.get_unit(a).unwrap().unwrap();
    assert_eq!(ua.lifecycle, Lifecycle::Deleted);
}

// ------------------------------------------------------------ lifecycle

fn a_source_change_marks_the_summary_stale_in_the_same_changeset<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(6));
    let (a, b, c) = (make(&db, b"aaaa1"), make(&db, b"bbbb1"), make(&db, b"cccc1"));
    let on_ab = db.build_summary([a, b], &trunc()).unwrap();
    let on_c = db.build_summary([c], &trunc()).unwrap();

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(b, UnitUpdate { payload: b"bbbb2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    assert_eq!(db.state(&on_ab), Some(DerivedState::Ready), "not before the commit");
    cs.commit().unwrap();
    assert_eq!(db.state(&on_ab), Some(DerivedState::Stale), "as soon as the commit returns");
    assert_eq!(db.summary(&on_ab), None, "a stale summary is not served");
    assert_eq!(db.state(&on_c), Some(DerivedState::Ready), "an unrelated summary is untouched");

    let rebuilt = db.rebuild_summary(&on_ab, &trunc()).unwrap();
    assert_eq!(db.summary(&rebuilt).as_deref(), Some(&b"aaaa\nbbbb"[..]));
    assert_eq!(db.descriptor(&rebuilt).unwrap().sources().last().unwrap().rev, RevNo(2));
}

fn a_new_method_or_version_creates_a_new_summary_and_supersedes_the_old<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(7));
    let a = make(&db, b"abcdefgh");
    let v1 = db.build_summary([a], &trunc()).unwrap();
    let v2 = db.supersede_summary(&v1, &TruncatingSummarizer::new(4, 2)).unwrap();
    assert_ne!(v1, v2);
    assert_eq!(db.state(&v1), Some(DerivedState::Superseded));
    assert_eq!(db.state(&v2), Some(DerivedState::Ready));
    assert_eq!(db.descriptor(&v2).unwrap().version(), 2);
    assert_eq!(db.descriptor(&v1).unwrap().version(), 1, "the old descriptor is not rewritten");

    let by_params = db.supersede_summary(&v2, &TruncatingSummarizer::new(6, 2)).unwrap();
    assert_eq!(db.summary(&by_params).as_deref(), Some(&b"abcdef"[..]));
    // A superseded summary cannot be superseded again, nor rebuilt.
    assert!(db.supersede_summary(&v1, &TruncatingSummarizer::new(1, 9)).is_err());
    assert!(db.rebuild_summary(&v1, &TruncatingSummarizer::new(4, 1)).is_err());
    // The same method, version and parameters over the same sources is the same summary.
    assert!(matches!(
        db.supersede_summary(&by_params, &TruncatingSummarizer::new(6, 2)),
        Err(ConsolidationError::Derived(DerivedStaleError::Derived(DerivedError::Duplicate(_))))
    ));
    assert_eq!(db.state(&by_params), Some(DerivedState::Ready));
}

fn rebuilding_needs_the_summarizer_that_made_the_summary<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(8));
    let a = make(&db, b"abcdefgh");
    let id = db.build_summary([a], &trunc()).unwrap();
    edit(&db, a, b"zzzzzzzz");
    for wrong in [TruncatingSummarizer::new(4, 2), TruncatingSummarizer::new(5, 1)] {
        assert!(matches!(db.rebuild_summary(&id, &wrong), Err(ConsolidationError::AdapterMismatch(i)) if i == id));
    }
    assert!(matches!(db.rebuild_summary(&id, &Renamed(trunc())), Err(ConsolidationError::AdapterMismatch(_))));
    assert!(matches!(db.rebuild_summary(&id, &Mute), Err(ConsolidationError::AdapterMismatch(_))));
    assert_eq!(db.state(&id), Some(DerivedState::Stale));
    assert!(db.rebuild_summary(&id, &trunc()).is_ok());
}

fn deleting_any_one_source_deletes_the_summary_in_the_same_changeset<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(9));
    let (a, b, c) = (make(&db, b"aaaa"), make(&db, b"bbbb"), make(&db, b"cccc"));
    let on_ab = db.build_summary([a, b], &trunc()).unwrap();
    let on_bc = db.build_summary([b, c], &trunc()).unwrap();
    let on_c = db.build_summary([c], &trunc()).unwrap();

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(a, None).unwrap();
    assert_eq!(db.state(&on_ab), Some(DerivedState::Ready), "not before the commit");
    cs.commit().unwrap();
    assert_eq!(db.state(&on_ab), None);
    assert_eq!(db.summary(&on_ab), None);
    assert_eq!(db.descriptor(&on_ab), None);
    assert_eq!(db.state(&on_bc), Some(DerivedState::Ready), "a summary of other sources stays");

    // The same for the second source of a two-source summary, whichever it is.
    delete(&db, c);
    assert_eq!(db.state(&on_bc), None);
    assert_eq!(db.state(&on_c), None);
    // The surviving source is intact.
    assert_eq!(db.get_unit(b).unwrap().unwrap().payload.as_deref(), Some(&b"bbbb"[..]));
}

fn a_stale_or_superseded_summary_is_deleted_with_its_source_too<E: Target>() {
    let db = fresh::<E>(&SimVfs::new(10));
    let a = make(&db, b"aaaa");
    let old = db.build_summary([a], &trunc()).unwrap();
    let new = db.supersede_summary(&old, &TruncatingSummarizer::new(2, 1)).unwrap();
    edit(&db, a, b"aaab");
    assert_eq!(db.state(&new), Some(DerivedState::Stale));
    delete(&db, a);
    assert_eq!(db.state(&old), None);
    assert_eq!(db.state(&new), None);
}

// ------------------------------------------------------------ reopening

fn summaries_are_rebuilt_on_open_from_their_sources<E: Target>() {
    let disk = SimVfs::new(11);
    let db = fresh::<E>(&disk);
    let (a, b) = (make(&db, b"aaaa1"), make(&db, b"bbbb1"));
    let id = db.build_summary([a, b], &trunc()).unwrap();
    let known = db.descriptor(&id).unwrap();
    db.close().unwrap();

    // Unchanged sources: recomputed, ready, same identity and text.
    let db = open_summaries(open_db::<E>(&disk), [known.clone()], &trunc()).unwrap().0;
    assert_eq!(db.state(&id), Some(DerivedState::Ready));
    assert_eq!(db.summary(&id).as_deref(), Some(&b"aaaa\nbbbb"[..]));
    edit(&db, a, b"aaaa2");
    db.close().unwrap();

    // The source moved while the summary was not in memory: stale, never ready.
    let db = open_summaries(open_db::<E>(&disk), [known.clone()], &trunc()).unwrap().0;
    assert_eq!(db.state(&id), Some(DerivedState::Stale));
    assert_eq!(db.summary(&id), None);

    // Only summaries made by the given summarizer are accepted.
    let other = db.build([a], "embed", 1, BTreeMap::new(), &plain).unwrap();
    let not_summary = db.descriptor(&other).unwrap();
    assert!(matches!(
        open_summaries(open_db::<E>(&SimVfs::new(12)), [not_summary], &trunc()),
        Err(ConsolidationError::NotASummary(_))
    ));
    assert!(matches!(
        open_summaries(open_db::<E>(&SimVfs::new(13)), [known], &TruncatingSummarizer::new(9, 1)),
        Err(ConsolidationError::AdapterMismatch(i)) if i == id
    ));
}

fn a_summary_leaves_nothing_on_disk<E: Target>() {
    let run = |with: bool| {
        let disk = SimVfs::new(14);
        let db = fresh::<E>(&disk);
        let (a, b) = (make(&db, b"aaaa1"), make(&db, b"bbbb1"));
        if with {
            let s = db.build_summary([a, b], &trunc()).unwrap();
            let s = db.supersede_summary(&s, &TruncatingSummarizer::new(2, 2)).unwrap();
            edit(&db, a, b"aaaa2");
            db.rebuild_summary(&s, &TruncatingSummarizer::new(2, 2)).unwrap();
        } else {
            edit(&db, a, b"aaaa2");
        }
        db.close().unwrap();
        (disk.dump(), disk)
    };
    let (with, _d1) = run(true);
    let (without, _d2) = run(false);
    assert_eq!(format!("{with:?}"), format!("{without:?}"), "summaries must not write a single byte");
}

// ------------------------------------------------------------ I-32

/// Every read of a source returns the same bytes with and without summaries:
/// two databases get the same writes, one of them with summaries built,
/// staled, rebuilt, superseded and deleted in between; then both are reopened
/// as plain databases and compared head by head and revision by revision.
fn reads_of_a_source_are_the_same_with_and_without_summaries<E: Target>() {
    let work = |disk: &SimVfs, with: bool| {
        let db = fresh::<E>(disk);
        let ids = [make(&db, b"one-1"), make(&db, b"two-1"), make(&db, b"three-1")];
        let mut last = None;
        if with {
            last = Some(db.build_summary(ids, &trunc()).unwrap());
            db.build_summary([ids[0]], &trunc()).unwrap();
        }
        edit(&db, ids[0], b"one-2");
        if let Some(s) = last {
            let s = db.rebuild_summary(&s, &trunc()).unwrap();
            db.supersede_summary(&s, &TruncatingSummarizer::new(1, 3)).unwrap();
        }
        edit(&db, ids[1], b"two-2");
        delete(&db, ids[2]);
        let reads = heads(&db, &ids);
        db.close().unwrap();
        (ids, reads)
    };
    let (d_with, d_without) = (SimVfs::new(15), SimVfs::new(15));
    let (ids, live_with) = work(&d_with, true);
    let (_, live_without) = work(&d_without, false);
    assert_eq!(live_with, live_without);

    let (db_with, db_without) = (open_db::<E>(&d_with), open_db::<E>(&d_without));
    for id in ids {
        assert_eq!(db_with.get_unit(id).unwrap(), db_without.get_unit(id).unwrap());
        let (h1, h2) = (
            db_with.history(id, HistoryQuery::oldest(100)).unwrap(),
            db_without.history(id, HistoryQuery::oldest(100)).unwrap(),
        );
        assert_eq!(h1, h2);
        for r in &h1 {
            assert_eq!(db_with.get_revision(id, r.id.rev).unwrap(), db_without.get_revision(id, r.id.rev).unwrap());
        }
    }
}

// ------------------------------------------------------------ failure

fn a_failed_commit_leaves_the_summaries_and_the_sources_as_they_were<E: Target>() {
    for what in ["update", "delete", "create"] {
        let disk = SimVfs::new(16);
        let db = fresh::<E>(&disk);
        let (a, b) = (make(&db, b"aaaa1"), make(&db, b"bbbb1"));
        let on_a = db.build_summary([a], &trunc()).unwrap();
        let on_ab = db.build_summary([a, b], &trunc()).unwrap();
        let before = heads(&db, &[a, b]);
        let text = db.summary(&on_ab);

        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        match what {
            "update" => {
                let upd = UnitUpdate { payload: b"aaaa2".to_vec(), schema_version: None, metadata: None };
                cs.update(a, upd, None).unwrap();
            }
            "delete" => {
                cs.delete(a, None).unwrap();
            }
            _ => {
                let new =
                    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"n".into() };
                cs.create(new).unwrap();
            }
        }
        disk.fail_when(|op| op.kind == OpKind::SyncData, 1, None);
        let result = cs.commit();
        assert!(disk.fault_injected(), "the fault did not fire ({what})");
        assert!(result.is_err(), "a failed data sync went unreported ({what})");

        // No summary lost, marked or added.
        assert_eq!(db.state(&on_a), Some(DerivedState::Ready), "{what}");
        assert_eq!(db.state(&on_ab), Some(DerivedState::Ready), "{what}");
        assert_eq!(db.summary(&on_ab), text, "{what}");

        // The sources on disk are as they were.
        let image = disk.crash_now(CrashKind::PowerLoss);
        let plain = open_db::<E>(&image);
        // The commit is all there or not at all (I-21): the untouched source is
        // as it was, and the touched one is as it was or as the commit left it.
        assert_eq!(plain.get_unit(b).unwrap(), before[1], "{what}");
        let ua = plain.get_unit(a).unwrap();
        let landed = match what {
            "update" => ua.as_ref().is_some_and(|u| u.payload.as_deref() == Some(&b"aaaa2"[..])),
            "delete" => ua.as_ref().is_some_and(|u| u.lifecycle == Lifecycle::Deleted),
            _ => false,
        };
        assert!(ua == before[0] || landed, "{what}");
    }
}

fn errors_say_what_happened_without_content<E: Target>() {
    let id = DerivedId([3; 32]);
    let all = [
        ConsolidationError::NoSources,
        ConsolidationError::ReservedParam,
        ConsolidationError::NotASummary(id),
        ConsolidationError::AdapterMismatch(id),
        ConsolidationError::Derived(DerivedStaleError::NoValidSources(id)),
    ];
    let texts: std::collections::BTreeSet<String> = all.iter().map(|e| e.to_string()).collect();
    assert_eq!(texts.len(), all.len(), "each error reads differently");
    assert!(texts.contains("a summary needs at least one source"));
    assert!(texts.contains("a summarizer parameter uses a reserved name"));
    assert!(texts.contains("the derived datum is not a summary"));
    assert!(texts.contains("the summarizer is not the one that made this summary"));
    assert!(texts.contains("no source of the derived datum is valid any more"), "a wrapped error keeps its text");
    let wrapped: ConsolidationError = DerivedStaleError::ComputeFailed(id).into();
    assert!(matches!(wrapped, ConsolidationError::Derived(DerivedStaleError::ComputeFailed(_))));
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn a_summary_records_its_exact_sources_and_a_full_descriptor() {
                super::a_summary_records_its_exact_sources_and_a_full_descriptor::<E>();
            }
            #[test]
            fn errors_say_what_happened_without_content() {
                super::errors_say_what_happened_without_content::<E>();
            }
            #[test]
            fn a_summary_with_no_sources_is_an_error() {
                super::a_summary_with_no_sources_is_an_error::<E>();
            }
            #[test]
            fn the_kind_parameter_is_reserved_and_separates_summaries() {
                super::the_kind_parameter_is_reserved_and_separates_summaries::<E>();
            }
            #[test]
            fn a_summarizer_that_gives_nothing_leaves_a_failed_summary() {
                super::a_summarizer_that_gives_nothing_leaves_a_failed_summary::<E>();
            }
            #[test]
            fn the_original_stays_readable_through_every_summary_operation() {
                super::the_original_stays_readable_through_every_summary_operation::<E>();
            }
            #[test]
            fn a_source_change_marks_the_summary_stale_in_the_same_changeset() {
                super::a_source_change_marks_the_summary_stale_in_the_same_changeset::<E>();
            }
            #[test]
            fn a_new_method_or_version_creates_a_new_summary_and_supersedes_the_old() {
                super::a_new_method_or_version_creates_a_new_summary_and_supersedes_the_old::<E>();
            }
            #[test]
            fn rebuilding_needs_the_summarizer_that_made_the_summary() {
                super::rebuilding_needs_the_summarizer_that_made_the_summary::<E>();
            }
            #[test]
            fn deleting_any_one_source_deletes_the_summary_in_the_same_changeset() {
                super::deleting_any_one_source_deletes_the_summary_in_the_same_changeset::<E>();
            }
            #[test]
            fn a_stale_or_superseded_summary_is_deleted_with_its_source_too() {
                super::a_stale_or_superseded_summary_is_deleted_with_its_source_too::<E>();
            }
            #[test]
            fn summaries_are_rebuilt_on_open_from_their_sources() {
                super::summaries_are_rebuilt_on_open_from_their_sources::<E>();
            }
            #[test]
            fn a_summary_leaves_nothing_on_disk() {
                super::a_summary_leaves_nothing_on_disk::<E>();
            }
            #[test]
            fn reads_of_a_source_are_the_same_with_and_without_summaries() {
                super::reads_of_a_source_are_the_same_with_and_without_summaries::<E>();
            }
            #[test]
            fn a_failed_commit_leaves_the_summaries_and_the_sources_as_they_were() {
                super::a_failed_commit_leaves_the_summaries_and_the_sources_as_they_were::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
