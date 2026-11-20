//! WAL shipping: the commits that follow the last shipped one, read from the write-ahead logs of
//! the live database and put in the store as segments.
//!
//! A [`Shipper`] only reads files, so it runs next to a database that a server holds open (and
//! under the engine's exclusive lock, which a second process cannot take). It ships what the logs
//! hold; a commit is in a log from the moment it is written until the engine has flushed it and
//! deleted the log, so a shipper that falls behind by more than that finds a hole ([`Error::Gap`])
//! and the answer is a new base backup, which [`Shipper::resume`] then continues from.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::base::hex;
use crate::catalog::catalog;
use crate::error::{Error, Result};
use crate::segment;
use crate::store::ObjectStore;
use crate::wal::{Commit, read_commits, read_db_id};

/// A segment holds commits up to about this many bytes (one commit larger than that is a segment
/// of its own).
const SEGMENT_BYTES: usize = 4 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ShipReport {
    /// Commits uploaded by this call.
    pub commits: u64,
    pub segments: u32,
    /// The last sequence the store holds a continuous chain up to.
    pub shipped_seq: u64,
}

pub struct Shipper {
    dir: PathBuf,
    store: Arc<dyn ObjectStore>,
    db_id: [u8; 16],
    shipped: u64,
}

impl Shipper {
    /// A shipper for the database in `dir` that continues the chain the store holds for it: from
    /// the newest base backup of this database, and on through the segments that follow it.
    ///
    /// # Errors
    ///
    /// [`Error::NoBase`] when the store has no base backup yet, [`Error::Refused`] when it only
    /// has backups of another database.
    pub fn resume(dir: &Path, store: Arc<dyn ObjectStore>) -> Result<Self> {
        let db_id = read_db_id(dir)?;
        let cat = catalog(&*store)?;
        let mine = hex(&db_id);
        let Some(base) = cat.bases.iter().rfind(|b| b.db_id == mine) else {
            return Err(if cat.bases.is_empty() {
                Error::NoBase("the store has no base backup: take one first".into())
            } else {
                Error::Refused("the store holds backups of another database".into())
            });
        };
        Ok(Shipper { dir: dir.to_owned(), store, db_id, shipped: cat.reach(base) })
    }

    /// The sequence the store holds a continuous chain up to.
    pub fn shipped_seq(&self) -> u64 {
        self.shipped
    }

    /// Ships every commit after [`Shipper::shipped_seq`] that the logs hold, stamping the segments
    /// with `now_ms`. A commit made before a point in time is found by a restore to that time only
    /// if it was shipped by then, so the time resolution of a restore is how often this runs.
    ///
    /// # Errors
    ///
    /// [`Error::Gap`] as described above; the store and the log files fail as they do. After an
    /// error the segments already uploaded stay, and the next call goes on from them.
    pub fn ship(&mut self, now_ms: u64) -> Result<ShipReport> {
        let (commits, db_id) = read_commits(&self.dir, self.shipped)?;
        if db_id.is_some_and(|id| id != self.db_id) {
            return Err(Error::Refused("the write-ahead logs belong to another database than the backups".into()));
        }
        let mut report = ShipReport { shipped_seq: self.shipped, ..ShipReport::default() };
        let mut rest = &commits[..];
        while !rest.is_empty() {
            let (mut n, mut bytes) = (0, 0);
            for c in rest {
                bytes += c.body.encoded_len();
                n += 1;
                if bytes >= SEGMENT_BYTES {
                    break;
                }
            }
            let (chunk, tail): (&[Commit], &[Commit]) = rest.split_at(n);
            let (first, last) = (chunk[0].seq, chunk[n - 1].seq);
            self.store.put(&segment::key(first, last, now_ms), &segment::encode(self.db_id, now_ms, chunk))?;
            self.shipped = last;
            report.commits += n as u64;
            report.segments += 1;
            rest = tail;
        }
        report.shipped_seq = self.shipped;
        Ok(report)
    }
}
