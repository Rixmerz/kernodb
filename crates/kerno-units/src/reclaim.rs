//! E9-07: public reclamation of live files (ADR-0009 option 3, point 2).
//!
//! `reclaim` asks the engine to compact the whole key space so that the live
//! files of the database stop holding keys and values that were deleted
//! logically. It is a mechanism, not a promise of physical or secure erasure:
//!
//! * Deleting is logical and takes effect at once (ADR-0009 point 1);
//!   `reclaim` only moves the moment at which the live files stop holding
//!   what was deleted.
//! * A file that an open snapshot taken before the deletion retains stays on
//!   disk until that snapshot is released (ADR-0009 point 2).
//! * The engine does not overwrite the blocks it frees, so nothing here says
//!   that the bytes leave the storage medium (ADR-0009 point 3, threat model
//!   §6.5).
//!
//! `reclaim` does not write to the layer: it commits no change set and
//! changes no visible state.

use std::ops::Bound;

use kerno_core::contract::{Reclaim, TxnEngine};

use crate::db::UnitDb;
use crate::derived_stale::DerivedDb;
use crate::error::UnitError;

impl<E: TxnEngine + Reclaim> UnitDb<E> {
    /// Asks the engine to compact the whole key space (units, revisions,
    /// relations and derived data), so that the live files stop holding what
    /// was deleted logically once no open snapshot retains it.
    ///
    /// This is a mechanism, not a promise of physical or secure erasure: a
    /// file retained by a snapshot taken before the deletion stays until the
    /// snapshot is released, and the engine does not overwrite the blocks it
    /// frees (ADR-0009 option 3). It changes no visible state and commits no
    /// change set. An engine error comes back as [`UnitError::Storage`].
    pub fn reclaim(&self) -> Result<(), UnitError> {
        self.check_poisoned()?;
        Reclaim::compact_range(self.engine(), Bound::Unbounded, Bound::Unbounded)?;
        Ok(())
    }
}

impl<E: TxnEngine + Reclaim> DerivedDb<E> {
    /// Reclaims the live files of the database underneath, derived data
    /// included. Same meaning as [`UnitDb::reclaim`]: a mechanism, not a
    /// promise of physical or secure erasure (ADR-0009 option 3).
    pub fn reclaim(&self) -> Result<(), UnitError> {
        self.db.reclaim()
    }
}
