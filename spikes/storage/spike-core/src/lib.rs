//! Shared pieces of the Kerno Phase 0 storage spike.
//!
//! The spike compares two candidate storage architectures (copy-on-write
//! B+ tree and LSM tree) behind one contract, so that both are exercised by
//! exactly the same functional, crash and corruption tests and by the same
//! benchmark workloads. Nothing in this crate is the final Kerno format.
//!
//! * [`engine`] — the contract every candidate implements.
//! * [`vfs`] — file-system abstraction, including [`vfs::SimVfs`], an
//!   in-memory file system that can simulate power loss and torn writes.
//! * [`harness`] — reusable test suites (see [`engine_test_suite!`]).

pub mod engine;
pub mod error;
pub mod harness;
pub mod reference;
pub mod rng;
pub mod vfs;

pub use engine::{Durability, Engine, MAX_KEY_LEN, MAX_VALUE_LEN, Op, Options, Snapshot, WriteBatch};
pub use error::{Error, Result};
pub use vfs::{File, OpenMode, Vfs};
