//! Compatibility shim for the frozen Kerno Phase 0 storage spikes.
//!
//! The contract, error model, write batches and VFS now live in `kerno-core`;
//! the shared test suites, the simulated VFS, the reference `LogEngine` and
//! the PRNG live in `kerno-testkit`. This crate only re-exports them under the
//! paths `spike-btree`, `spike-lsm` and `spike-bench` have always used, so the
//! M0 evidence keeps compiling and running unchanged. It contains no code of
//! its own.

pub use kerno_core::contract::{Engine, Options, Snapshot};
pub use kerno_core::vfs::{File, OpenMode, Vfs};
pub use kerno_core::{Durability, Error, MAX_KEY_LEN, MAX_VALUE_LEN, Op, Result, WriteBatch};
pub use kerno_testkit::{engine_test_suite, harness, reference, rng};

/// File-system abstraction ([`kerno_core::vfs`]) plus the simulated and
/// counting file systems from `kerno-testkit`.
pub mod vfs {
    pub use kerno_core::vfs::*;
    pub use kerno_testkit::vfs::{CountingVfs, CrashKind, IoCounters, IoSnapshot, SimVfs};
}

/// The harness contract and write batches (formerly `spike_core::engine`).
pub mod engine {
    pub use kerno_core::contract::*;
    pub use kerno_core::{MAX_KEY_LEN, MAX_VALUE_LEN, Op, WriteBatch};
}

/// The error model (formerly `spike_core::error`).
pub mod error {
    pub use kerno_core::{Error, Result};
}
