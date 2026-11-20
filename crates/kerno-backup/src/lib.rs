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

/// The store a URL names: `file:///absolute/dir`, or `s3://bucket/prefix` (S3 and R2; see
/// [`s3::S3Store::from_env`]).
///
/// # Errors
///
/// An unknown scheme, or an S3 store whose environment is incomplete.
/// The local path of the part of a `file://` URL after the scheme. On Windows `file:///C:/dir`
/// names `C:/dir`: the slash before a drive letter is dropped (issue #4).
fn local_path(path: &str, windows: bool) -> &str {
    let b = path.as_bytes();
    if windows && b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        &path[1..]
    } else {
        path
    }
}

pub fn open_store(url: &str) -> Result<Arc<dyn ObjectStore>> {
    if let Some(path) = url.strip_prefix("file://") {
        if !path.starts_with('/') {
            return Err(Error::Store(format!("{url:?}: a file:// URL needs an absolute path (file:///dir)")));
        }
        Ok(Arc::new(FsStore::new(local_path(path, cfg!(windows)))))
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
    use super::local_path;

    #[test]
    fn a_drive_letter_loses_its_leading_slash_on_windows_only() {
        assert_eq!(local_path("/C:/backups/db", true), "C:/backups/db");
        assert_eq!(local_path("/C:/backups/db", false), "/C:/backups/db");
        assert_eq!(local_path("/srv/backups", true), "/srv/backups");
        assert_eq!(local_path("/srv/backups", false), "/srv/backups");
    }
}
