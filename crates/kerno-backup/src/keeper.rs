//! The loop a server runs: keep shipping, and take a new base backup whenever the chain needs one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kerno_storage::KernoDb;
use kerno_units::UnitDb;

use crate::base::{BaseInfo, backup_base};
use crate::error::{Error, Result};
use crate::ship::{ShipReport, Shipper};
use crate::store::ObjectStore;

/// What one [`Keeper::tick`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// The base backup it had to take (the first one, or after a gap), if any.
    pub base: Option<BaseInfo>,
    pub shipped: ShipReport,
}

/// Keeps the backups of one open database up to date.
pub struct Keeper {
    dir: PathBuf,
    store: Arc<dyn ObjectStore>,
    shipper: Option<Shipper>,
}

impl Keeper {
    pub fn new(dir: &Path, store: Arc<dyn ObjectStore>) -> Self {
        Keeper { dir: dir.to_owned(), store, shipper: None }
    }

    /// Takes a base backup now, and ships from it on.
    pub fn base_now(&mut self, db: &UnitDb<KernoDb>, now_ms: u64) -> Result<BaseInfo> {
        self.shipper = None;
        let base = backup_base(&self.dir, db, &*self.store, now_ms)?;
        self.shipper = Some(Shipper::resume(&self.dir, Arc::clone(&self.store))?);
        Ok(base)
    }

    /// Ships what is new. With no base backup in the store, or with a gap in the logs (the shipper
    /// came too late for a log the engine had already deleted), it takes a base first.
    ///
    /// # Errors
    ///
    /// As [`backup_base`] and [`Shipper::ship`], except the two cases above.
    pub fn tick(&mut self, db: &UnitDb<KernoDb>, now_ms: u64) -> Result<Tick> {
        let mut base = None;
        if self.shipper.is_none() {
            match Shipper::resume(&self.dir, Arc::clone(&self.store)) {
                Ok(s) => self.shipper = Some(s),
                Err(Error::NoBase(_)) => base = Some(self.base_now(db, now_ms)?),
                Err(e) => return Err(e),
            }
        }
        for attempt in 0..2 {
            let Some(shipper) = self.shipper.as_mut() else { return Err(Error::NoBase("no shipper".into())) };
            match shipper.ship(now_ms) {
                Ok(shipped) => return Ok(Tick { base, shipped }),
                Err(Error::Gap { .. }) if attempt == 0 => base = Some(self.base_now(db, now_ms)?),
                Err(e) => return Err(e),
            }
        }
        Err(Error::NoBase("a gap right after a new base backup".into()))
    }
}
