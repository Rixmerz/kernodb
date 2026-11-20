//! E9-07: public reclamation of live files (ADR-0009 option 3, point 2).
//!
//! `reclaim` is a mechanism, not a promise of physical or secure erasure. The
//! tests measure only what ADR-0009 measures: which live files hold a
//! sentinel after `reclaim` and a clean close (T-RE1), and with a snapshot
//! taken before the delete (T-RE2). Nothing is said about the storage medium.
//!
//! The rules run on both engines. `Emulated<LogEngine>` has no `Reclaim` of
//! its own, so it runs behind `Probe`, a delegating engine that records the
//! range it is asked to compact; `Probe` hands out nothing of the engine it
//! wraps. The file measurement reads the files `KernoDb` writes, so it runs
//! on `kerno_db` only, as in `derived_suppression.rs`. The raw store is read
//! by opening the engine on its own after the layer is closed, never through
//! the layer.

mod harness;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use harness::units::{Dump, Target, dump, open_units};
use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Reclaim, Snapshot, Txn, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::{OpKind, SimVfs};
use kerno_units::vector_store::VectorRecipe;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;
type Range = (Bound<Vec<u8>>, Bound<Vec<u8>>);

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

/// The reference engine plus a `Reclaim` that records the range it is asked
/// for and can be told to fail.
struct Probe {
    inner: EmuLog,
}

thread_local! {
    /// The ranges `Probe` was asked to compact, and whether it refuses. Per
    /// thread, so each test owns its own: the layer does not hand out the
    /// engine it guards.
    static CALLS: RefCell<Vec<Range>> = const { RefCell::new(Vec::new()) };
    static REFUSE: Cell<bool> = const { Cell::new(false) };
}

impl Engine for Probe {
    type Snapshot = <EmuLog as Engine>::Snapshot;

    fn name() -> &'static str {
        "probe"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> kerno_core::Result<Self> {
        Ok(Probe { inner: EmuLog::open(vfs, dir, opts)? })
    }

    fn commit(&self, batch: &kerno_core::WriteBatch) -> kerno_core::Result<u64> {
        self.inner.commit(batch)
    }

    fn snapshot(&self) -> kerno_core::Result<Self::Snapshot> {
        self.inner.snapshot()
    }

    fn close(self) -> kerno_core::Result<()> {
        self.inner.close()
    }

    fn stats(&self) -> Vec<(String, u64)> {
        self.inner.stats()
    }
}

impl TxnEngine for Probe {
    type Txn<'a> = <EmuLog as TxnEngine>::Txn<'a>;

    fn begin_write(&self) -> kerno_core::Result<Self::Txn<'_>> {
        self.inner.begin_write()
    }
}

impl Reclaim for Probe {
    fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> kerno_core::Result<()> {
        if REFUSE.get() {
            return Err(kerno_core::Error::InvalidArgument("probe: compaction refused".into()));
        }
        CALLS.with_borrow_mut(|c| c.push((start.map(<[u8]>::to_vec), end.map(<[u8]>::to_vec))));
        Ok(())
    }
}

impl Target for Probe {
    const NAME: &'static str = "probe";
    fn open_engine(vfs: Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<Self> {
        Probe::open(vfs, Path::new(harness::units::DIR), opts)
    }
}

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open_db<E: Target>(disk: &SimVfs) -> UnitDb<E> {
    open_units::<E>(disk.as_vfs(), &cfg()).expect("open")
}

fn open_dd<E: Target>(disk: &SimVfs) -> DerivedDb<E> {
    DerivedDb::open_persisted(open_db::<E>(disk)).expect("open_persisted").0
}

/// The raw store and its commit sequence, read by opening the engine on its
/// own; the layer on `disk` must be closed.
fn stored<E: Target>(disk: &SimVfs) -> (Dump, u64) {
    let engine = E::open_engine(disk.as_vfs(), &cfg().engine).expect("open engine");
    let snap = engine.snapshot().expect("snapshot");
    let out = (dump(&snap), snap.seq());
    drop(snap);
    engine.close().expect("close engine");
    out
}

fn recipe() -> VectorRecipe {
    VectorRecipe { method: "embed".into(), version: 1, profile: "model-a".into(), params: BTreeMap::new() }
}

fn make<E: Target>(dd: &DerivedDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn delete<E: Target>(dd: &DerivedDb<E>, unit: UnitId) {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(unit, None).unwrap();
    cs.commit().unwrap();
}

fn vcompute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<f32>> {
    inputs.first().map(|i| i.payload.iter().map(|b| f32::from(*b)).collect())
}

/// The sentinel of the vector: the bits of two f32 that spell it.
const SENTINEL: &[u8; 8] = b"VSENT-RC";

fn sentinel_vector(_: &DerivedDescriptor, _: &[SourceInput]) -> Option<Vec<f32>> {
    Some(SENTINEL.as_chunks::<4>().0.iter().map(|c| f32::from_bits(u32::from_be_bytes(*c))).collect())
}

/// The live files that hold the sentinel, by name; an unlinked file still
/// open counts as one.
fn holders(disk: &SimVfs) -> BTreeSet<String> {
    let has = |h: &[u8]| h.windows(SENTINEL.len()).any(|w| w == SENTINEL);
    let image = disk.dump();
    let mut out: BTreeSet<String> = image
        .files
        .iter()
        .filter(|(_, bytes)| has(bytes))
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    if image.unlinked_open.iter().any(|b| has(b)) {
        out.insert("(unlinked, open)".into());
    }
    out
}

fn live_files(disk: &SimVfs) -> BTreeSet<String> {
    disk.dump().files.keys().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
}

// ------------------------------------------------------------------ rules

/// I-42 with `reclaim`: the change set that deleted a source keeps its
/// derived data invisible, and `reclaim` writes nothing the layer can see:
/// the same keys and values, the same commit sequence, no new change set.
fn reclaim_changes_no_visible_state<E: Target + Reclaim>() {
    let disk = SimVfs::new(90);
    let dd = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"alpha"), make(&dd, b"beta"));
    let va = dd.build_vector(a, &recipe(), ChangeMeta::default(), &vcompute).unwrap();
    let vb = dd.build_vector(b, &recipe(), ChangeMeta::default(), &vcompute).unwrap();
    delete(&dd, a);
    dd.close().unwrap();
    let before = stored::<E>(&disk);

    let db = open_db::<E>(&disk);
    db.reclaim().unwrap();
    let dd = DerivedDb::open_persisted(db).expect("open_persisted").0;
    dd.reclaim().unwrap();
    assert!(dd.vector(&va).is_none(), "I-42: the derived datum of the deleted source is visible after reclaim");
    assert_eq!(dd.state(&vb), Some(DerivedState::Ready));
    assert!(dd.vector(&vb).is_some());
    assert!(dd.get_unit(a).unwrap().is_none_or(|u| u.lifecycle != Lifecycle::Active));
    assert!(dd.get_unit(b).unwrap().is_some());
    dd.close().unwrap();
    assert_eq!(stored::<E>(&disk), before, "reclaim changed the stored records or the commit sequence");
}

/// I-1 and I-2 with `reclaim`: a snapshot taken before the delete keeps
/// reading the unit after `reclaim`; later reads do not. Everything read
/// right before `reclaim` (the unit, its vector and state, and the unit at
/// the snapshot) reads the same right after it. On `KernoDb` the files are
/// measured as well (T-RE1, T-RE2): with the snapshot open, the live files
/// that hold the sentinel are among the files that existed when it was
/// taken; after it is released and the database is closed, none. Without
/// the file measurement (`emulated_log`), the test shows only that `reclaim`
/// returns `Ok` and leaves the logical content unchanged.
fn reclaim_keeps_what_a_snapshot_retains<E: Target + Reclaim>(hold: bool, measure_files: bool) {
    let disk = SimVfs::new(93 + u64::from(hold));
    let dd = open_dd::<E>(&disk);
    let u = make(&dd, b"hello");
    let id = dd.build_vector(u, &recipe(), ChangeMeta::default(), &sentinel_vector).unwrap();
    dd.close().unwrap();
    if measure_files {
        assert!(!holders(&disk).is_empty(), "test setup: the sentinel is on disk before the delete");
    }

    let db = open_db::<E>(&disk);
    let held = hold.then(|| (db.snapshot().unwrap(), live_files(&disk)));
    let dd = DerivedDb::open_persisted(db).expect("open_persisted").0;
    delete(&dd, u);
    let read = || {
        let at_snapshot = held.as_ref().map(|(s, _)| s.get_unit(u).unwrap());
        (dd.get_unit(u).unwrap(), dd.vector(&id), dd.state(&id), at_snapshot)
    };
    let before = read();
    dd.reclaim().unwrap();
    assert_eq!(read(), before, "reclaim changed the logical content");
    assert!(dd.vector(&id).is_none(), "I-1: the vector is visible after the delete");
    assert!(dd.get_unit(u).unwrap().is_none_or(|x| x.lifecycle != Lifecycle::Active), "I-1: the unit is visible");
    if let Some((snap, at_snapshot)) = held {
        let unit = snap.get_unit(u).unwrap().expect("I-2: the earlier snapshot no longer reads the unit");
        assert_eq!(unit.lifecycle, Lifecycle::Active, "I-2: the earlier snapshot saw the delete");
        if measure_files {
            let left = holders(&disk);
            assert!(!left.is_empty(), "T-RE2: the files the snapshot retains left while it was open");
            assert!(left.is_subset(&at_snapshot), "T-RE2: {left:?} hold the sentinel, retained: {at_snapshot:?}");
        }
        drop(snap);
    }
    dd.close().unwrap();
    if measure_files {
        let left = holders(&disk);
        assert!(left.is_empty(), "T-RE1, hold={hold}: live files hold the sentinel after reclaim and close: {left:?}");
    }
}

macro_rules! engines {
    ($($m:ident: $e:ty, $files:expr);*) => {$(
        mod $m {
            #[test]
            fn reclaim_changes_no_visible_state() {
                super::reclaim_changes_no_visible_state::<$e>();
            }

            #[test]
            fn reclaim_retires_live_files_without_a_snapshot() {
                super::reclaim_keeps_what_a_snapshot_retains::<$e>(false, $files);
            }

            #[test]
            fn reclaim_keeps_what_a_snapshot_retains() {
                super::reclaim_keeps_what_a_snapshot_retains::<$e>(true, $files);
            }
        }
    )*};
}

engines!(emulated_log: crate::Probe, false; kerno_db: kerno_storage::KernoDb, true);

// ------------------------------------------------------------ the engine call

/// Both entry points ask the engine for the whole key space, once each.
#[test]
fn reclaim_asks_for_the_whole_key_space() {
    let disk = SimVfs::new(91);
    let db = open_db::<Probe>(&disk);
    db.reclaim().unwrap();
    let dd = DerivedDb::open_persisted(db).expect("open_persisted").0;
    dd.reclaim().unwrap();
    assert_eq!(CALLS.take(), vec![(Bound::Unbounded, Bound::Unbounded); 2]);
    dd.close().unwrap();
}

/// An engine error is the caller's error, as a storage error, from both
/// entry points.
#[test]
fn reclaim_reports_an_engine_failure() {
    let disk = SimVfs::new(92);
    let db = open_db::<Probe>(&disk);
    REFUSE.set(true);
    assert!(matches!(db.reclaim(), Err(UnitError::Storage(_))));
    let dd = DerivedDb::open_persisted(db).expect("open_persisted").0;
    assert!(matches!(dd.reclaim(), Err(UnitError::Storage(_))));
    REFUSE.set(false);
    dd.reclaim().unwrap();
    dd.close().unwrap();
}

/// The same rule on `KernoDb`: a failed write while the files are compacted
/// comes back as a storage error, not as success.
#[test]
fn kerno_db_reclaim_reports_an_engine_failure() {
    let disk = SimVfs::new(96);
    let dd = open_dd::<KernoDb>(&disk);
    make(&dd, b"alpha");
    disk.fail_when(|op| op.kind == OpKind::SyncData, 1, None);
    let result = dd.reclaim();
    assert!(disk.fault_injected(), "test setup: the fault did not fire");
    assert!(matches!(result, Err(UnitError::Storage(_))), "{result:?}");
}

/// The trait's default `compact_range`, on a type that names `Reclaim`
/// without implementing it, answers "unsupported" (R-E9-17).
#[test]
fn the_default_compact_range_is_unsupported() {
    struct Bare;
    impl Reclaim for Bare {}
    let err = Bare.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap_err();
    assert!(matches!(&err, kerno_core::Error::InvalidArgument(m) if m == "unsupported"), "{err:?}");
}

// ---------------------------------------------------------------- compact

/// An engine whose transactions report a read sequence one too high, so the
/// layer's prediction misses the engine's real commit sequence and the first
/// change set poisons the handle. Everything else, `Reclaim` included, is the
/// wrapped engine's.
struct Skew<B>(B);
struct SkewTxn<T>(T);

impl<B: TxnEngine> Engine for Skew<B> {
    type Snapshot = B::Snapshot;

    fn name() -> &'static str {
        "skew"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> kerno_core::Result<Self> {
        Ok(Skew(B::open(vfs, dir, opts)?))
    }

    fn commit(&self, batch: &WriteBatch) -> kerno_core::Result<u64> {
        self.0.commit(batch)
    }

    fn snapshot(&self) -> kerno_core::Result<Self::Snapshot> {
        self.0.snapshot()
    }

    fn close(self) -> kerno_core::Result<()> {
        self.0.close()
    }

    fn stats(&self) -> Vec<(String, u64)> {
        self.0.stats()
    }
}

impl<B: TxnEngine> TxnEngine for Skew<B> {
    type Txn<'a>
        = SkewTxn<B::Txn<'a>>
    where
        Self: 'a;

    fn begin_write(&self) -> kerno_core::Result<Self::Txn<'_>> {
        Ok(SkewTxn(self.0.begin_write()?))
    }
}

impl<B: Reclaim> Reclaim for Skew<B> {
    fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> kerno_core::Result<()> {
        self.0.compact_range(start, end)
    }
}

impl<B: Target> Target for Skew<B> {
    const NAME: &'static str = "skew";
    fn open_engine(vfs: Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<Self> {
        Ok(Skew(B::open_engine(vfs, opts)?))
    }
}

impl<T: Txn> Txn for SkewTxn<T> {
    fn read_seq(&self) -> u64 {
        self.0.read_seq() + 1
    }
    fn get(&self, key: &[u8]) -> kerno_core::Result<Option<Vec<u8>>> {
        self.0.get(key)
    }
    fn scan(&self, s: Bound<&[u8]>, e: Bound<&[u8]>, limit: usize) -> kerno_core::Result<Dump> {
        self.0.scan(s, e, limit)
    }
    fn put(&mut self, key: &[u8], value: &[u8]) -> kerno_core::Result<()> {
        self.0.put(key, value)
    }
    fn delete(&mut self, key: &[u8]) -> kerno_core::Result<()> {
        self.0.delete(key)
    }
    fn delete_range(&mut self, s: &[u8], e: &[u8]) -> kerno_core::Result<()> {
        self.0.delete_range(s, e)
    }
    fn commit(self) -> kerno_core::Result<u64> {
        self.0.commit()
    }
}

/// `compact` asks the engine for the revision ranges only: from `REV_META`
/// (included) to just past `REV_PAYLOAD` (excluded), design §5.1.
#[test]
fn compact_asks_for_the_revision_ranges() {
    let disk = SimVfs::new(97);
    let db = open_db::<Probe>(&disk);
    db.compact().unwrap();
    assert_eq!(CALLS.take(), vec![(Bound::Included(vec![0x20]), Bound::Excluded(vec![0x22]))]);
    db.close().unwrap();
}

/// A poisoned handle refuses `compact` before the engine is asked for
/// anything, on both engines.
fn compact_refuses_a_poisoned_handle<E: Target + Reclaim>() {
    let disk = SimVfs::new(98);
    let db = open_db::<Skew<E>>(&disk);
    let first = db.create_unit(
        NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"x".to_vec() },
        ChangeMeta::default(),
    );
    assert!(matches!(first, Err(UnitError::Storage(kerno_core::Error::Poisoned(_)))), "setup: {first:?}");
    CALLS.take();
    let r = db.compact();
    assert!(matches!(r, Err(UnitError::Storage(kerno_core::Error::Poisoned(_)))), "got {r:?}");
    assert!(CALLS.take().is_empty(), "a refused compact reached the engine");
}

#[test]
fn compact_refuses_a_poisoned_handle_emulated_log() {
    compact_refuses_a_poisoned_handle::<Probe>();
}

#[test]
fn compact_refuses_a_poisoned_handle_kerno_db() {
    compact_refuses_a_poisoned_handle::<KernoDb>();
}
