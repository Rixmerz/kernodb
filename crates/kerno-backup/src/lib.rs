#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Hot backup, point-in-time restore and WAL shipping of a Kerno database to object storage. See
//! `docs/backup.md`.
//!
//! A backup is a **base** ([`backup_base`]): every raw key and value at one commit sequence, read
//! from a snapshot while the database keeps writing; and **segments** ([`Shipper`]): the commits
//! that follow, read from the write-ahead logs. [`restore`] puts a base and the segments after it
//! back into a new directory, to the latest commit, to an exact one or to a moment in time.
//!
//! Backups live in an [`ObjectStore`]: a directory ([`FsStore`]), S3 or R2 ([`s3::S3Store`]), or
//! memory for tests ([`MemStore`]).

mod base;
mod catalog;
mod error;
mod keeper;
mod object;
mod restore;
pub mod s3;
mod segment;
mod ship;
mod store;
mod wal;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub use base::{BaseInfo, backup_base};
pub use catalog::{Catalog, SegmentInfo, catalog};
pub use error::{Error, Result};
pub use keeper::{Keeper, Tick};
pub use restore::{Restored, Target, restore};
pub use ship::{ShipReport, Shipper};
pub use store::{FsStore, MemStore, ObjectStore, Prefixed};

/// Milliseconds since the Unix epoch, for stamping what is uploaded.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The local path a `file://` URL names, given what follows the scheme; `None` when that is not an
/// absolute path. A Unix path starts with `/` (`file:///srv/x`). On Windows the URL form of
/// `C:\x` is `file:///C:/x`, with the slash before the drive letter dropped (issue #4), and what
/// you get by writing `file://` before a native path, `file://C:\x` or `file://C:/x`, is taken to
/// mean the same drive path: with no host before it, a drive letter cannot be anything else.
fn local_path(rest: &str, windows: bool) -> Option<&str> {
    let drive_path = |p: &str| {
        let b = p.as_bytes();
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\')
    };
    if windows && drive_path(rest) {
        Some(rest)
    } else if windows && rest.starts_with('/') && drive_path(&rest[1..]) {
        Some(&rest[1..])
    } else {
        rest.starts_with('/').then_some(rest)
    }
}

/// The store a URL names: `file:///absolute/dir` (`file:///C:/dir` on Windows), or
/// `s3://bucket/prefix` (S3 and R2; see [`s3::S3Store::from_env`]).
///
/// # Errors
///
/// An unknown scheme, a `file://` URL that is not an absolute path, or an S3 store whose
/// environment is incomplete.
pub fn open_store(url: &str) -> Result<Arc<dyn ObjectStore>> {
    if let Some(rest) = url.strip_prefix("file://") {
        let Some(path) = local_path(rest, cfg!(windows)) else {
            return Err(Error::Store(format!(
                "{url:?}: a file:// URL needs an absolute path (file:///dir, or file:///C:/dir on Windows)"
            )));
        };
        Ok(Arc::new(FsStore::new(path)))
    } else if let Some(rest) = url.strip_prefix("s3://") {
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        let s3 = s3::S3Store::from_env(bucket)?;
        Ok(Arc::new(Prefixed::new(Arc::new(s3), prefix)))
    } else {
        Err(Error::Store(format!("{url:?}: the backup location is file:///dir or s3://bucket/prefix")))
    }
}

#[cfg(test)]
mod file_url_tests {
    use super::{local_path, open_store};

    #[test]
    fn a_drive_letter_loses_its_leading_slash_on_windows_only() {
        assert_eq!(local_path("/C:/backups/db", true), Some("C:/backups/db"));
        assert_eq!(local_path("/C:/backups/db", false), Some("/C:/backups/db"));
        assert_eq!(local_path("/C:\\backups\\db", true), Some("C:\\backups\\db"));
        assert_eq!(local_path("/srv/backups", true), Some("/srv/backups"));
        assert_eq!(local_path("/srv/backups", false), Some("/srv/backups"));
    }

    #[test]
    fn a_native_windows_path_after_the_scheme_means_that_path_on_windows_only() {
        // `file://` + `C:\x`, as `format!("file://{}", path.display())` writes it.
        assert_eq!(local_path("C:\\backups\\db", true), Some("C:\\backups\\db"));
        assert_eq!(local_path("c:/backups/db", true), Some("c:/backups/db"));
        assert_eq!(local_path("C:\\backups\\db", false), None);
        assert_eq!(local_path("c:/backups/db", false), None);
    }

    #[test]
    fn a_path_that_is_not_absolute_is_refused_everywhere() {
        for windows in [false, true] {
            for rest in ["", "relative", "relative/dir", "C:", "C:dir", "C:relative\\dir", "1:/dir", "host/dir"] {
                assert_eq!(local_path(rest, windows), None, "{rest:?} windows={windows}");
            }
        }
        // A bare `/C:` is a directory named `C:` at the root, not a drive.
        assert_eq!(local_path("/C:", true), Some("/C:"));
    }

    #[test]
    fn open_store_says_what_a_file_url_needs() {
        let err = open_store("file://relative").err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("a file:// URL needs an absolute path"), "{err}");
        assert!(open_store("file:///some/dir").is_ok());
        assert!(open_store("ftp://host/x").is_err());
    }
}
