//! The one error type of the crate.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// The object store failed: a file system or network error, with the key it was about.
    Store(String),
    /// A backup object is damaged or is not one: a wrong digest, a short body, an unknown version.
    /// A restore stops here; it never produces a smaller database.
    Corrupt(String),
    /// The engine failed while reading or writing a database.
    Engine(kerno_core::Error),
    /// The units layer refused to hand out a snapshot (a poisoned database).
    Layer(String),
    /// The commits the shipper needs are gone from the write-ahead logs (the engine deleted a log
    /// after flushing it before it was shipped): the chain in the store has a hole, so a new base
    /// backup has to be taken.
    Gap { expected: u64, found: u64 },
    /// There is no base backup in the store yet (or none that the target allows).
    NoBase(String),
    /// The store ends before the point in time that was asked for.
    Unreachable { wanted: u64, reached: u64 },
    /// The restore destination is not empty, or the store belongs to another database.
    Refused(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Store(m) | Error::Corrupt(m) | Error::Layer(m) | Error::NoBase(m) | Error::Refused(m) => {
                f.write_str(m)
            }
            Error::Engine(e) => write!(f, "{e}"),
            Error::Gap { expected, found } => write!(
                f,
                "the write-ahead log no longer holds commit {expected} (the oldest it has is {found}): take a new base backup"
            ),
            Error::Unreachable { wanted, reached } => {
                write!(f, "the backup reaches commit {reached} only, not {wanted}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<kerno_core::Error> for Error {
    fn from(e: kerno_core::Error) -> Self {
        Error::Engine(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Store(e.to_string())
    }
}
