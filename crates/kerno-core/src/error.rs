use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors are matched with `matches!` outside this crate: new variants are
/// not a breaking change.
#[derive(Debug)]
#[non_exhaustive]
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
    /// The files were written by a version or with a feature this engine does
    /// not support (a newer `format_version`, an unknown incompatible feature
    /// bit, encryption, the M0 prototype format). It is not damage: nothing
    /// was modified, and a newer engine may read them.
    Incompatible(String),
    /// A write was attempted through a handle that only reads.
    ReadOnly(String),
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
            Error::Incompatible(m) => write!(f, "incompatible format: {m}"),
            Error::ReadOnly(m) => write!(f, "read-only: {m}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_variants_display_their_message() {
        assert_eq!(
            Error::Incompatible("M0 prototype format".into()).to_string(),
            "incompatible format: M0 prototype format"
        );
        assert_eq!(Error::ReadOnly("no writes".into()).to_string(), "read-only: no writes");
        assert!(std::error::Error::source(&Error::Incompatible("x".into())).is_none());
    }
}
