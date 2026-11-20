//! What a store holds: its base backups and its WAL segments, and whether they chain.

use serde::Serialize;

use crate::base::{BaseInfo, read_manifest};
use crate::error::{Error, Result};
use crate::segment;
use crate::store::ObjectStore;

/// One shipped segment, as its key names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SegmentInfo {
    pub key: String,
    pub first: u64,
    pub last: u64,
    pub observed_ms: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Catalog {
    /// By sequence, oldest first.
    pub bases: Vec<BaseInfo>,
    /// By first sequence.
    pub segments: Vec<SegmentInfo>,
}

impl Catalog {
    /// The last sequence a restore from `base` can reach: the segments that follow it one after the
    /// other, and no further than the first hole.
    pub fn reach(&self, base: &BaseInfo) -> u64 {
        let mut reached = base.seq;
        for s in &self.segments {
            if s.first > reached + 1 {
                break;
            }
            reached = reached.max(s.last);
        }
        reached
    }
}

pub fn catalog(store: &dyn ObjectStore) -> Result<Catalog> {
    let mut cat = Catalog::default();
    for key in store.list("base/")? {
        let Some(seq) = key.strip_suffix("/MANIFEST").and_then(|k| k.strip_prefix("base/")) else { continue };
        let seq = seq.parse::<u64>().map_err(|_| Error::Corrupt(format!("{key} is not a base manifest")))?;
        cat.bases.push(read_manifest(store, seq)?);
    }
    for key in store.list("wal/")? {
        let (first, last, observed_ms) =
            segment::parse_key(&key).ok_or_else(|| Error::Corrupt(format!("{key} is not a WAL segment")))?;
        cat.segments.push(SegmentInfo { key, first, last, observed_ms });
    }
    cat.bases.sort_by_key(|b| b.seq);
    Ok(cat)
}
