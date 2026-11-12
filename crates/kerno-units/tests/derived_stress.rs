//! E9-12: stress of derived data (etapa9-acta.md §3 E9-12, T-ST1, F-09 in part).
//!
//! Four writer threads each own a source unit and a derived datum over it.
//! A cycle edits the source and rebuilds the datum. Two reader threads poll
//! while the writers run. Invariant: no datum is `ready` (nor has a value)
//! once a change to its source has returned. No time threshold, and nothing
//! waits on a clock: the readers poll a counter and yield.

mod harness;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use harness::units::{Target, open_units};
use kerno_core::contract::{Options, TxnEngine};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const WRITERS: usize = 4;
const READERS: usize = 2;
/// 4 writers x 50 cycles = the 200 cycles of T-ST1.
const CYCLES: usize = 50;

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn compute(d: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<u8>> {
    let mut out = format!("{}@{}:", d.method(), d.version()).into_bytes();
    for i in inputs {
        out.extend_from_slice(&i.payload);
        out.push(b'|');
    }
    Some(out)
}

fn payload(writer: usize, cycle: usize) -> Vec<u8> {
    format!("w{writer}-c{cycle}").into_bytes()
}

fn expected(writer: usize, cycle: usize) -> Vec<u8> {
    let mut out = b"embed@1:".to_vec();
    out.extend_from_slice(&payload(writer, cycle));
    out.push(b'|');
    out
}

/// Per writer: the datums it built, and how many edits have returned.
struct Lane {
    ids: Mutex<Vec<DerivedId>>,
    edits: AtomicUsize,
}

struct Finish<'a> {
    finished: &'a AtomicUsize,
    done: &'a AtomicBool,
}

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        if self.finished.fetch_add(1, Ordering::AcqRel) + 1 == WRITERS {
            self.done.store(true, Ordering::Release);
        }
    }
}

fn make<E: TxnEngine>(db: &DerivedDb<E>, payload: Vec<u8>) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload }).unwrap();
    cs.commit().unwrap();
    id
}

fn edit<E: TxnEngine>(db: &DerivedDb<E>, id: UnitId, payload: Vec<u8>) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload, schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

fn no_ready_datum_has_a_changed_source<E: Target + Send + Sync>() {
    let db: DerivedDb<E> =
        DerivedDb::open(open_units::<E>(SimVfs::new(1).as_vfs(), &cfg()).expect("open"), Vec::new(), &compute)
            .unwrap()
            .0;

    let sources: Vec<UnitId> = (0..WRITERS).map(|w| make(&db, payload(w, 0))).collect();
    let lanes: Vec<Lane> = (0..WRITERS)
        .map(|w| {
            let id = db.build([sources[w]], "embed", 1, BTreeMap::new(), &compute).unwrap();
            Lane { ids: Mutex::new(vec![id]), edits: AtomicUsize::new(0) }
        })
        .collect();
    let finished = AtomicUsize::new(0);
    let done = AtomicBool::new(false);
    let violations = AtomicUsize::new(0);
    let reads = AtomicUsize::new(0);

    std::thread::scope(|s| {
        for w in 0..WRITERS {
            let (db, lanes, sources, finished, violations, done) =
                (&db, &lanes, &sources, &finished, &violations, &done);
            s.spawn(move || {
                // Counts the writer as finished even if it panics, so the
                // readers never poll forever.
                let _finish = Finish { finished, done };
                for k in 0..CYCLES {
                    let old = lanes[w].ids.lock().unwrap()[k];
                    edit(db, sources[w], payload(w, k + 1));
                    lanes[w].edits.store(k + 1, Ordering::Release);
                    // The writer is the only one that changes its source.
                    if db.state(&old) == Some(DerivedState::Ready) || db.value(&old).is_some() {
                        violations.fetch_add(1, Ordering::Relaxed);
                    }
                    let new = db.rebuild(&old, &compute).unwrap();
                    if db.state(&new) != Some(DerivedState::Ready) || db.value(&new) != Some(expected(w, k + 1)) {
                        violations.fetch_add(1, Ordering::Relaxed);
                    }
                    lanes[w].ids.lock().unwrap().push(new);
                }
            });
        }
        for _ in 0..READERS {
            let (db, lanes, violations, reads, done) = (&db, &lanes, &violations, &reads, &done);
            s.spawn(move || {
                loop {
                    let last = done.load(Ordering::Acquire);
                    for (w, lane) in lanes.iter().enumerate() {
                        // Edits that have returned are read first; the ids
                        // are pushed before the edit that supersedes them.
                        let returned = lane.edits.load(Ordering::Acquire);
                        let ids = lane.ids.lock().unwrap().clone();
                        for id in &ids[..returned] {
                            if db.state(id) == Some(DerivedState::Ready) || db.value(id).is_some() {
                                violations.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        // A value served is the one of its own revision.
                        if let Some(v) = db.value(&ids[returned.min(ids.len() - 1)]) {
                            let at = returned.min(ids.len() - 1);
                            if v != expected(w, at) {
                                violations.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        reads.fetch_add(1, Ordering::Relaxed);
                    }
                    if last {
                        break;
                    }
                    std::thread::yield_now();
                }
            });
        }
    });

    assert_eq!(violations.load(Ordering::Relaxed), 0, "invariant violations");
    assert!(reads.load(Ordering::Relaxed) >= READERS * WRITERS, "the readers ran");
    for (w, lane) in lanes.iter().enumerate() {
        let ids = lane.ids.lock().unwrap();
        assert_eq!(ids.len(), CYCLES + 1);
        assert_eq!(db.value(&ids[CYCLES]), Some(expected(w, CYCLES)), "the last datum is ready and current");
    }
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn no_ready_datum_has_a_changed_source() {
                super::no_ready_datum_has_a_changed_source::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
