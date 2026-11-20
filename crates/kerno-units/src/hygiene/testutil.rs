//! Test fixtures: a database over the emulated VFS (runs on macOS and Linux),
//! with a clock the test moves and counter ids.

use std::path::Path;
use std::sync::Arc;

use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;

use crate::clock::{Clock, CounterIds, FixedClock};
use crate::db::{ChangeMeta, NewUnit, UnitConfig, UnitDb};
use crate::types::{Timestamp, UnitId};

pub type Engine = Emulated<LogEngine>;

pub const T0: Timestamp = Timestamp(1_700_000_000_000_000);
pub const HOUR: i64 = 3_600_000_000;

pub struct Fixture {
    pub db: UnitDb<Engine>,
    pub clock: Arc<FixedClock>,
    _disk: SimVfs,
}

pub fn fixture() -> Fixture {
    let disk = SimVfs::new(0);
    let clock = Arc::new(FixedClock::new(T0));
    let cfg = UnitConfig::with_sources(clock.clone(), Arc::new(CounterIds::new(1)));
    let db = UnitDb::<Engine>::open(disk.as_vfs(), Path::new("/db"), &cfg).expect("open layer");
    Fixture { db, clock, _disk: disk }
}

impl Fixture {
    /// Creates a unit of `unit_type` with `payload` and `metadata`.
    pub fn make(&self, unit_type: &str, payload: &[u8], metadata: Vec<u8>) -> UnitId {
        let new = NewUnit { unit_type: unit_type.into(), schema_version: 1, metadata, payload: payload.to_vec() };
        self.db.create_unit(new, ChangeMeta::default()).expect("create").0
    }

    pub fn advance(&self, micros: i64) {
        self.clock.set(Timestamp(self.clock.now().0 + micros));
    }
}
