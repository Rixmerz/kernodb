//! Test kit for Kerno storage engines.
//!
//! Every engine that implements the harness contract
//! ([`kerno_core::contract::Engine`]) is exercised by exactly the same
//! functional, crash and corruption tests.
//!
//! * [`vfs`] — the [`kerno_core::vfs`] abstraction plus [`vfs::SimVfs`], an
//!   in-memory file system that can simulate power loss and torn writes (and,
//!   opt-in, Linux `fsync` semantics, per-directory durability, a full disk,
//!   filtered triggers and traces), [`vfs::CountingVfs`], an I/O accounting
//!   wrapper, and [`vfs::HookVfs`], a fault-injecting wrapper.
//! * [`harness`] — reusable test suites (see [`engine_test_suite!`]).
//! * [`reference`](mod@reference) — `LogEngine`, the trivially correct engine
//!   that validates the harness itself.
//! * [`rng`] — the deterministic PRNG every randomized test is seeded with.
//! * [`process`] — re-executes the test binary as a child process.

pub mod harness;
pub mod process;
pub mod reference;
pub mod rng;

pub mod vfs {
    //! The [`kerno_core::vfs`] abstraction plus the simulated and counting
    //! file systems used by the tests.

    mod counting;
    mod hook;
    mod sim;

    pub use counting::*;
    pub use hook::*;
    pub use kerno_core::vfs::*;
    pub use sim::*;
}

pub use kerno_core::contract::{Engine, Options, Snapshot};
pub use kerno_core::{Durability, Error, MAX_KEY_LEN, MAX_VALUE_LEN, Op, Result, WriteBatch};
