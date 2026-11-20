//! Restoring a database from a store, to its latest state or to a point in time.
//!
//! The units layer writes the commit sequence of every change into its records and expects the next
//! commit to get the next sequence, so a restored engine has to stand at exactly the sequence the
//! backup was taken at, not just hold the same keys. The restore therefore spends sequences the way
//! the original did: it starts with one no-op commit for each sequence the base's own commits do
//! not account for, writes the base in one commit per part, and then replays the shipped commits one
//! engine commit each. Restoring takes time proportional to the number of commits the database ever
//! had, not to its size.

use std::path::{Path, PathBuf};

use kerno_core::WriteBatch;
use kerno_core::contract::Durability;
use kerno_core::vfs::StdVfs;
use kerno_storage::format::wal::CanonicalCommit;
use kerno_storage::{Config, KernoDb};

use crate::base::{BaseInfo, read_part};
use crate::catalog::catalog;
use crate::error::{Error, Result};
use crate::segment;
use crate::store::ObjectStore;
use crate::wal::PAD_KEY;

/// How far to restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// As far as the store goes.
    Latest,
    /// Exactly to this commit sequence, or an error when the store does not reach it.
    Seq(u64),
    /// To the state the shipper had seen at this moment (milliseconds since the Unix epoch): the
    /// newest base taken by then, and the segments shipped by then.
    Time(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Restored {
    /// The sequence of the base backup the restore started from.
    pub base_seq: u64,
    /// The sequence the restored database stands at.
    pub seq: u64,
    /// Commits replayed from segments after the base.
    pub replayed: u64,
}

fn batch_of(commit: &CanonicalCommit<Vec<u8>>) -> WriteBatch {
    let mut batch = WriteBatch::new();
    if commit.is_empty() {
        // A commit that changed nothing still had a sequence number.
        batch.delete(PAD_KEY.to_vec());
    }
    for (start, end) in &commit.ranges {
        batch.delete_range(start.clone(), end.clone());
    }
    for (key, value) in &commit.points {
        match value {
            Some(v) => batch.put(key.clone(), v.clone()),
            None => batch.delete(key.clone()),
        };
    }
    batch
}

/// The engine's next commit, checked to have the sequence it has to have.
fn commit(db: &KernoDb, batch: &WriteBatch, seq: &mut u64) -> Result<()> {
    use kerno_core::contract::Engine;
    let got = db.commit(batch)?;
    *seq += 1;
    if got != *seq {
        return Err(Error::Corrupt(format!("the restored engine committed sequence {got}, expected {seq}")));
    }
    Ok(())
}

fn staging_of(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().map_or_else(|| "database".into(), std::ffi::OsStr::to_os_string);
    name.push(".restoring");
    dir.with_file_name(name)
}

/// Restores the database `store` holds into `dir`, which must not exist or be empty.
///
/// It is built in `<dir>.restoring` and moved into place when it is whole, so a failed restore
/// leaves `dir` as it was.
///
/// # Errors
///
/// [`Error::Refused`] for a destination that is in use; [`Error::NoBase`] when no base backup
/// fits the target; [`Error::Unreachable`] when the store ends before a [`Target::Seq`];
/// [`Error::Corrupt`] for a damaged or missing object.
pub fn restore(store: &dyn ObjectStore, dir: &Path, target: Target) -> Result<Restored> {
    let in_use = match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(Error::Store(format!("cannot read {}: {e}", dir.display()))),
    };
    if in_use {
        return Err(Error::Refused(format!("{} is not empty: restore into a new directory", dir.display())));
    }
    let staging = staging_of(dir);
    if staging.exists() {
        return Err(Error::Refused(format!("{} is left by an earlier restore: remove it first", staging.display())));
    }
    let built = build(store, &staging, target);
    match built {
        Ok(restored) => {
            // An empty directory that is already there is replaced.
            let _ = std::fs::remove_dir(dir);
            std::fs::rename(&staging, dir)
                .map_err(|e| Error::Store(format!("cannot move the restore into place: {e}")))?;
            Ok(restored)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

fn pick_base(bases: &[BaseInfo], target: Target) -> Result<&BaseInfo> {
    let found = match target {
        Target::Latest => bases.last(),
        Target::Seq(n) => bases.iter().rfind(|b| b.seq <= n),
        Target::Time(t) => bases.iter().rfind(|b| b.created_ms <= t),
    };
    found.ok_or_else(|| {
        Error::NoBase(match target {
            Target::Latest => "the store has no base backup".into(),
            Target::Seq(n) => format!("the store has no base backup at or before commit {n}"),
            Target::Time(t) => format!("the store has no base backup taken at or before {t} ms"),
        })
    })
}

fn build(store: &dyn ObjectStore, staging: &Path, target: Target) -> Result<Restored> {
    let cat = catalog(store)?;
    let base = pick_base(&cat.bases, target)?;
    let limit = if let Target::Seq(n) = target { n } else { u64::MAX };

    // The segments that continue the base, up to the target.
    let mut expected = base.seq + 1;
    let mut chosen = Vec::new();
    for s in &cat.segments {
        let too_late = matches!(target, Target::Time(t) if s.observed_ms > t);
        if s.last < expected {
            continue;
        }
        if s.first > expected || expected > limit || too_late {
            break;
        }
        chosen.push(s);
        expected = s.last + 1;
    }
    // A segment past a hole in the chain is data the restore would silently leave out.
    if let Some(s) = cat.segments.iter().find(|s| {
        let too_late = matches!(target, Target::Time(t) if s.observed_ms > t);
        s.last >= expected && s.first > expected && expected <= limit && !too_late
    }) {
        return Err(Error::Corrupt(format!(
            "the chain of segments has a hole: commits {expected} to {} are missing",
            s.first - 1
        )));
    }
    let reached = (expected - 1).min(limit);
    if let Target::Seq(n) = target
        && reached < n
    {
        return Err(Error::Unreachable { wanted: n, reached });
    }

    let config = Config { durability: Durability::Buffered, allow_unverified_platform: true, ..Config::default() };
    let db = KernoDb::open(StdVfs::shared(), staging, &config)?;
    let mut at = 0u64;

    // One no-op commit for every sequence the base's parts do not stand for.
    let parts = u64::from(base.parts);
    if parts > base.seq {
        return Err(Error::Refused(format!(
            "the base at commit {} has {parts} parts: more than commits to write them with",
            base.seq
        )));
    }
    let mut pad = WriteBatch::new();
    pad.delete(PAD_KEY.to_vec());
    for _ in 0..base.seq - parts {
        commit(&db, &pad, &mut at)?;
    }
    for n in 0..base.parts {
        let mut batch = WriteBatch::new();
        for (k, v) in read_part(store, base.seq, n)? {
            batch.put(k, v);
        }
        commit(&db, &batch, &mut at)?;
    }

    let mut replayed = 0;
    for s in chosen {
        let bytes = store.get(&s.key)?.ok_or_else(|| Error::Corrupt(format!("{} is missing", s.key)))?;
        let seg = segment::decode(&bytes, &s.key)?;
        if seg.observed_ms != s.observed_ms {
            return Err(Error::Corrupt(format!("{} is not the object its key names", s.key)));
        }
        if crate::base::hex(&seg.db_id) != base.db_id {
            return Err(Error::Refused(format!("{} belongs to another database than the base", s.key)));
        }
        for c in seg.commits {
            if c.seq <= at {
                continue;
            }
            if c.seq > reached {
                break;
            }
            commit(&db, &batch_of(&c.body), &mut at)?;
            replayed += 1;
        }
    }
    if at != reached {
        return Err(Error::Corrupt(format!("the restore ended at commit {at}, not {reached}")));
    }
    db.close()?;
    Ok(Restored { base_seq: base.seq, seq: at, replayed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_that_changed_nothing_still_spends_a_sequence() {
        use kerno_core::contract::Engine;
        let dir = std::env::temp_dir().join(format!("kb-empty-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config { allow_unverified_platform: true, ..Config::default() };
        let db = KernoDb::open(StdVfs::shared(), &dir, &config).unwrap();
        let batch = batch_of(&CanonicalCommit::default());
        assert_eq!(db.commit(&batch).unwrap(), 1);
        assert_eq!(db.commit(&batch).unwrap(), 2);
        drop(db);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
