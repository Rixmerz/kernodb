//! E9-06: the draft export does not depend on the physical format (I-30,
//! T-EX2), over `Emulated<LogEngine>` and `KernoDb`.
//!
//! The same history is exported at one logical state while the files under it
//! change: before and after a flush of the write buffers, after the public
//! `reclaim` (a directed compaction of the whole key space), and after a clean
//! close and reopen. The bytes are compared for equality, never for time. On
//! `KernoDb` the files are also seen to change, so equal bytes are not equal
//! because nothing moved. The same history on the other engine gives the same
//! bytes too.
//!
//! The flush and the compaction reach the engine through `Hooked`, a
//! delegating engine that keeps a weak handle to what it wraps for the test's
//! `flush`; it hands out nothing of the engine it guards and lets go of it at
//! close. `Emulated<LogEngine>` has no files, so for it both are no-ops and
//! only the close and the cross-engine comparison say something.

mod harness;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::Path;
use std::sync::{Arc, Weak};

use harness::units::{Target, open_units};
use kerno_core::contract::{Engine, Options, Reclaim, TestHooks, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::export::export;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

// ------------------------------------------------------------ the engines

/// What the test asks of an engine's files.
trait Physical {
    /// Writes the write buffers to files.
    fn flush(&self) -> kerno_core::Result<()>;
    /// Compacts the files that overlap the range.
    fn compact(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> kerno_core::Result<()>;
}

impl Physical for KernoDb {
    fn flush(&self) -> kerno_core::Result<()> {
        TestHooks::flush(self)
    }
    fn compact(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> kerno_core::Result<()> {
        Reclaim::compact_range(self, start, end)
    }
}

impl Physical for EmuLog {
    fn flush(&self) -> kerno_core::Result<()> {
        Ok(())
    }
    fn compact(&self, _: Bound<&[u8]>, _: Bound<&[u8]>) -> kerno_core::Result<()> {
        Ok(())
    }
}

thread_local! {
    /// The engine the last `Hooked` opened on this thread. Weak, so the
    /// engine can still be closed by value; per thread, so each test owns its
    /// own.
    static OPEN: RefCell<Option<Weak<dyn Physical>>> = const { RefCell::new(None) };
}

/// Flushes the files of the engine open on this thread.
fn flush_files() {
    let engine = OPEN.with_borrow(|w| w.as_ref().and_then(Weak::upgrade)).expect("an engine is open");
    engine.flush().expect("flush");
}

struct Hooked<B> {
    inner: Arc<B>,
}

impl<B: Target + Physical> Engine for Hooked<B> {
    type Snapshot = B::Snapshot;

    fn name() -> &'static str {
        "hooked"
    }

    fn open(vfs: Arc<dyn Vfs>, _dir: &Path, opts: &Options) -> kerno_core::Result<Self> {
        <Self as Target>::open_engine(vfs, opts)
    }

    fn commit(&self, batch: &kerno_core::WriteBatch) -> kerno_core::Result<u64> {
        self.inner.commit(batch)
    }

    fn snapshot(&self) -> kerno_core::Result<Self::Snapshot> {
        self.inner.snapshot()
    }

    fn close(self) -> kerno_core::Result<()> {
        match Arc::try_unwrap(self.inner) {
            Ok(engine) => engine.close(),
            Err(_) => Err(kerno_core::Error::InvalidArgument("engine still shared at close".into())),
        }
    }

    fn stats(&self) -> Vec<(String, u64)> {
        self.inner.stats()
    }
}

impl<B: Target + Physical> TxnEngine for Hooked<B> {
    type Txn<'a> = B::Txn<'a>;

    fn begin_write(&self) -> kerno_core::Result<Self::Txn<'_>> {
        self.inner.begin_write()
    }
}

impl<B: Target + Physical> Reclaim for Hooked<B> {
    fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> kerno_core::Result<()> {
        self.inner.compact(start, end)
    }
}

impl<B: Target + Physical> Target for Hooked<B> {
    const NAME: &'static str = B::NAME;

    fn open_engine(vfs: Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<Self> {
        let inner = Arc::new(B::open_engine(vfs, opts)?);
        let weak: Weak<B> = Arc::downgrade(&inner);
        OPEN.with_borrow_mut(|w| *w = Some(weak));
        Ok(Hooked { inner })
    }
}

// ------------------------------------------------------------ the history

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
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

/// Every revision kind, policies, one relation of each provenance, a delete.
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
    cs.delete(d, None).unwrap();
    cs.commit().unwrap();
    vec![a, b, c, d]
}

/// One more change on top of [`populate`].
fn more<E: TxnEngine>(db: &UnitDb<E>, ids: &[UnitId]) {
    let mut cs = db.begin_changeset(by(b"eve", b"cli")).unwrap();
    cs.update(ids[0], upd(b"a-3"), None).unwrap();
    cs.commit().unwrap();
}

/// The files under the database: name and size.
fn files(disk: &SimVfs) -> BTreeMap<String, usize> {
    disk.dump().files.iter().map(|(p, b)| (p.file_name().unwrap().to_string_lossy().into_owned(), b.len())).collect()
}

// ------------------------------------------------------------ the rules

fn physical_independence<B: Target + Physical>(files_move: bool) {
    // The same history on the other engine, never flushed: the bytes to match.
    let reference: UnitDb<EmuLog> =
        UnitDb::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &cfg()).expect("open reference");
    let ids = populate(&reference);
    let draft = export(&reference).unwrap();
    more(&reference, &ids);
    let draft_more = export(&reference).unwrap();
    assert_ne!(draft, draft_more, "test setup: the second change must show in the export");

    let disk = SimVfs::new(60);
    let db = open_units::<Hooked<B>>(disk.as_vfs(), &cfg()).expect("open");
    let ids = populate(&db);
    assert_eq!(export(&db).unwrap(), draft, "the engine changes the draft");
    let at_first = files(&disk);

    // After a flush of the write buffers.
    flush_files();
    assert_eq!(export(&db).unwrap(), draft, "the draft changed after a flush");
    let flushed = files(&disk);
    if files_move {
        assert_ne!(flushed, at_first, "test setup: the flush did not touch the files");
    }

    // After the public reclaim (a compaction of the whole key space).
    db.reclaim().unwrap();
    assert_eq!(export(&db).unwrap(), draft, "the draft changed after reclaim");
    let reclaimed = files(&disk);
    if files_move {
        assert_ne!(reclaimed, flushed, "test setup: reclaim did not touch the files");
    }

    // Further writes after both, then flush and reclaim again.
    more(&db, &ids);
    assert_eq!(export(&db).unwrap(), draft_more);
    flush_files();
    db.reclaim().unwrap();
    assert_eq!(export(&db).unwrap(), draft_more, "the draft changed after a second flush and reclaim");

    // After a clean close and a reopen.
    let before_close = files(&disk);
    db.close().unwrap();
    if files_move {
        assert_ne!(files(&disk), before_close, "test setup: the close did not touch the files");
    }
    let db = open_units::<Hooked<B>>(disk.as_vfs(), &cfg()).expect("reopen");
    assert_eq!(export(&db).unwrap(), draft_more, "the draft changed after a close");
    db.reclaim().unwrap();
    assert_eq!(export(&db).unwrap(), draft_more, "the draft changed after reclaim on a reopened database");
    db.close().unwrap();
}

#[test]
fn physical_independence_emulated_log() {
    physical_independence::<EmuLog>(false);
}

#[test]
fn physical_independence_kerno_db() {
    physical_independence::<KernoDb>(true);
}

/// The draft of a database that was flushed, reclaimed and closed imports
/// into a new database and exports the same bytes: the files carry nothing
/// the draft needs.
fn reimport_after_physical_changes<B: Target + Physical>() {
    let disk = SimVfs::new(61);
    let db = open_units::<Hooked<B>>(disk.as_vfs(), &cfg()).expect("open");
    populate(&db);
    flush_files();
    db.reclaim().unwrap();
    let draft = export(&db).unwrap();
    db.close().unwrap();

    let fresh = open_units::<Hooked<B>>(SimVfs::new(62).as_vfs(), &cfg()).expect("open fresh");
    kerno_units::import::import(&fresh, &draft).unwrap();
    assert_eq!(export(&fresh).unwrap(), draft);
    flush_files();
    fresh.reclaim().unwrap();
    assert_eq!(export(&fresh).unwrap(), draft);
    fresh.close().unwrap();
}

#[test]
fn reimport_after_physical_changes_emulated_log() {
    reimport_after_physical_changes::<EmuLog>();
}

#[test]
fn reimport_after_physical_changes_kerno_db() {
    reimport_after_physical_changes::<KernoDb>();
}
