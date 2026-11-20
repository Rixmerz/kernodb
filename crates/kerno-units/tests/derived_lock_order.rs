//! F-09: the lock order of the guarded commit of derived data.
//!
//! `DerivedChangeSet::commit` takes the registry lock before the engine
//! commits and holds it until the stale marks are made (I-23). A reader that
//! takes the same lock therefore never sees the new source with a registry
//! still behind it. Showing that with a racing reader cannot be made
//! deterministic, so the engine is wrapped: at the moment the engine's own
//! commit runs, on the committing thread, it asks the registry whether its
//! lock is held. A lock taken after the commit answers `false` there.
#![cfg(feature = "test-support")]

mod harness;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use harness::units::{Dump, Target, open_units};
use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Txn, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

thread_local! {
    /// What to ask when the wrapped engine commits a transaction, and the
    /// answers it got. Per thread, so each test owns its own.
    static ASK: RefCell<Option<Box<dyn Fn() -> bool>>> = const { RefCell::new(None) };
    static ANSWERS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

/// An engine that, before it commits a transaction, runs `ASK` and records
/// the answer. Everything else is the wrapped engine's.
struct Peek<B>(B);
struct PeekTxn<T>(T);

impl<B: TxnEngine> Engine for Peek<B> {
    type Snapshot = B::Snapshot;

    fn name() -> &'static str {
        "peek"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> kerno_core::Result<Self> {
        Ok(Peek(B::open(vfs, dir, opts)?))
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
}

impl<B: TxnEngine> TxnEngine for Peek<B> {
    type Txn<'a>
        = PeekTxn<B::Txn<'a>>
    where
        Self: 'a;

    fn begin_write(&self) -> kerno_core::Result<Self::Txn<'_>> {
        Ok(PeekTxn(self.0.begin_write()?))
    }
}

impl<B: Target> Target for Peek<B> {
    const NAME: &'static str = "peek";
    fn open_engine(vfs: Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<Self> {
        Ok(Peek(B::open_engine(vfs, opts)?))
    }
}

impl<T: Txn> Txn for PeekTxn<T> {
    fn read_seq(&self) -> u64 {
        self.0.read_seq()
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
        if let Some(answer) = ASK.with_borrow(|ask| ask.as_ref().map(|f| f())) {
            ANSWERS.with_borrow_mut(|a| a.push(answer));
        }
        self.0.commit()
    }
}

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn compute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<u8>> {
    inputs.first().map(|i| i.payload.clone())
}

fn registry_is_locked_while_the_engine_commits<E: Target>() {
    let db = open_units::<Peek<E>>(SimVfs::new(1).as_vfs(), &cfg()).expect("open");
    let dd = Arc::new(DerivedDb::open(db, Vec::new(), &compute).unwrap().0);

    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let unit = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"v1".to_vec() })
        .unwrap();
    cs.commit().unwrap();
    let id = dd.build([unit], "embed", 1, BTreeMap::new(), &compute).unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Ready));
    assert!(!dd.hook_registry_locked(), "setup: nobody holds the registry");

    let asked = Arc::clone(&dd);
    ASK.set(Some(Box::new(move || asked.hook_registry_locked())));
    ANSWERS.take();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(unit, UnitUpdate { payload: b"v2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    ASK.set(None);

    // One engine commit, and the registry was already locked when it ran.
    assert_eq!(ANSWERS.take(), vec![true], "the registry lock is taken before the engine commits");
    // The stale mark was made under that lock, and the lock is released.
    assert_eq!(dd.state(&id), Some(DerivedState::Stale));
    assert!(!dd.hook_registry_locked(), "the lock is released when the commit returns");
}

#[test]
fn emulated_log_registry_is_locked_while_the_engine_commits() {
    registry_is_locked_while_the_engine_commits::<EmuLog>();
}

#[test]
fn kerno_db_registry_is_locked_while_the_engine_commits() {
    registry_is_locked_while_the_engine_commits::<KernoDb>();
}
