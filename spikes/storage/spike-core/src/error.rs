use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// Underlying I/O failure.
    Io(io::Error),
    /// Stored data failed validation (checksum, structure, bounds).
    /// Engines must return this instead of returning damaged data.
    Corruption(String),
    /// The database is already open by another handle or process.
    Locked,
    /// The caller passed something the engine does not accept
    /// (e.g. a key longer than [`crate::MAX_KEY_LEN`]).
    InvalidArgument(String),
    /// A previous fatal error (typically a failed `fsync`) left the handle
    /// in an unknown state. The database must be reopened, which runs
    /// recovery from what is actually on disk.
    Poisoned(String),
}

impl Error {
    pub fn corruption(msg: impl Into<String>) -> Self {
        Error::Corruption(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Corruption(m) => write!(f, "corruption detected: {m}"),
            Error::Locked => write!(f, "database is locked by another handle"),
            Error::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            Error::Poisoned(m) => write!(f, "handle poisoned by an earlier fatal error: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
