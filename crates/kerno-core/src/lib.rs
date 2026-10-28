//! Product-owned contract of the Kerno storage engine: error model, write
//! batches, durability modes and the file-system abstraction.
//!
//! * [`Error`] and [`Result`] — the error taxonomy every engine returns.
//! * [`WriteBatch`] and [`Op`] — atomic groups of writes and their limits.
//! * [`vfs`] — file-system abstraction; every file access of an engine goes
//!   through [`vfs::Vfs`] so that crashes and corruption can be simulated.
//!
//! The harness contract (`Engine`, `Snapshot`, `Options`) lives in the hidden
//! `contract` module: it is what the shared test suites in `kerno-testkit`
//! exercise, not a public API.

mod batch;
mod error;

pub use batch::{MAX_KEY_LEN, MAX_VALUE_LEN, Op, WriteBatch};
pub use contract::Durability;
pub use error::{Error, Result};

#[doc(hidden)]
pub mod contract;
pub mod vfs;
