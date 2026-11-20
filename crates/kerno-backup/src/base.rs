//! A base backup: every key and value of the engine at one sequence number, taken from a snapshot
//! while the database keeps serving writes.
//!
//! The engine hands out a snapshot as a view at one commit sequence, so reading it page by page is
//! consistent however much is committed meanwhile. The copy is of raw keys and values, below the
//! units layer: it carries every index, count and derived record exactly as stored, and a restore
//! needs no knowledge of any of them.
//!
//! Layout: `base/{seq}/part-{n}` objects, then `base/{seq}/MANIFEST`. The manifest is written
//! last, so a base that was cut short is not one: nothing lists it and nothing restores from it.

use std::ops::Bound;
use std::path::Path;

use kerno_core::contract::Snapshot;
use kerno_storage::KernoDb;
use kerno_units::UnitDb;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::object::{Cursor, Kind, seal, unseal};
use crate::store::ObjectStore;
use crate::wal::{PAD_KEY, read_db_id};

/// A part holds at most this many bytes of keys and values and this many entries: below what one
/// engine commit takes (64 MiB, 2^20 operations), because a restore writes one commit per part.
const PART_BYTES: usize = 48 << 20;
const PART_OPS: usize = 1 << 20;
/// Entries read from the snapshot at a time.
const PAGE: usize = 4096;

/// What `base/{seq}/MANIFEST` says about a base backup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseInfo {
    /// The engine sequence the base is the state at.
    pub seq: u64,
    /// The `db_id` of the database it was taken from, in hex.
    pub db_id: String,
    /// When it was taken, in milliseconds since the Unix epoch.
    pub created_ms: u64,
    pub parts: u32,
    pub entries: u64,
    /// Bytes of keys and values.
    pub bytes: u64,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn manifest_key(seq: u64) -> String {
    format!("base/{seq:020}/MANIFEST")
}

pub fn part_key(seq: u64, n: u32) -> String {
    format!("base/{seq:020}/part-{n:06}")
}

pub fn read_manifest(store: &dyn ObjectStore, seq: u64) -> Result<BaseInfo> {
    let key = manifest_key(seq);
    let bytes = store.get(&key)?.ok_or_else(|| Error::Corrupt(format!("{key} is missing")))?;
    let body = unseal(Kind::BaseManifest, &bytes, &key)?;
    let info: BaseInfo = serde_json::from_slice(body).map_err(|e| Error::Corrupt(format!("{key}: {e}")))?;
    if info.seq != seq {
        return Err(Error::Corrupt(format!("{key} says it is the base at {}", info.seq)));
    }
    Ok(info)
}

/// The entries of a part object: `(key, value)` pairs.
pub fn read_part(store: &dyn ObjectStore, seq: u64, n: u32) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let key = part_key(seq, n);
    let bytes = store.get(&key)?.ok_or_else(|| Error::Corrupt(format!("{key} is missing")))?;
    let body = unseal(Kind::BasePart, &bytes, &key)?;
    let mut c = Cursor::new(body, &key);
    let count = c.u32()?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let klen = c.u32()? as usize;
        let k = c.take(klen)?.to_vec();
        let vlen = c.u32()? as usize;
        entries.push((k, c.take(vlen)?.to_vec()));
    }
    if !c.is_empty() {
        return Err(Error::Corrupt(format!("{key}: bytes left after the last entry")));
    }
    Ok(entries)
}

struct Part {
    entries: Vec<(Vec<u8>, Vec<u8>)>,
    bytes: usize,
}

impl Part {
    fn seal(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.bytes + 8 * self.entries.len() + 4);
        body.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (k, v) in &self.entries {
            body.extend_from_slice(&(k.len() as u32).to_le_bytes());
            body.extend_from_slice(k);
            body.extend_from_slice(&(v.len() as u32).to_le_bytes());
            body.extend_from_slice(v);
        }
        seal(Kind::BasePart, &body)
    }
}

/// Copies the database of `dir`, open as `db` (read-only is enough), into `store`, and returns what it made.
///
/// The writes that arrive while it runs are not in it (they are in the WAL segments the shipper
/// uploads from the sequence after [`BaseInfo::seq`]). `dir` is only read, for the `db_id` that
/// ties the base to its database.
///
/// # Errors
///
/// The snapshot or the store fails, or the database holds the key [`PAD_KEY`] that this crate
/// reserves.
pub fn backup_base(dir: &Path, db: &UnitDb<KernoDb>, store: &dyn ObjectStore, now_ms: u64) -> Result<BaseInfo> {
    let db_id = read_db_id(dir)?;
    let snap = db.raw_snapshot().map_err(|e| Error::Layer(e.to_string()))?;
    let seq = snap.seq();
    let mut info = BaseInfo { seq, db_id: hex(&db_id), created_ms: now_ms, parts: 0, entries: 0, bytes: 0 };
    let mut part = Part { entries: Vec::new(), bytes: 0 };
    let mut after: Option<Vec<u8>> = None;
    loop {
        let start = after.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
        let page = snap.scan(start, Bound::Unbounded, PAGE)?;
        let done = page.len() < PAGE;
        for (k, v) in page {
            if k == PAD_KEY {
                return Err(Error::Refused("the database holds the key this crate reserves for restores".into()));
            }
            after = Some(k.clone());
            part.bytes += k.len() + v.len();
            info.bytes += (k.len() + v.len()) as u64;
            info.entries += 1;
            part.entries.push((k, v));
            if part.bytes >= PART_BYTES || part.entries.len() >= PART_OPS {
                store.put(&part_key(seq, info.parts), &part.seal())?;
                info.parts += 1;
                part = Part { entries: Vec::new(), bytes: 0 };
            }
        }
        if done {
            break;
        }
    }
    if !part.entries.is_empty() {
        store.put(&part_key(seq, info.parts), &part.seal())?;
        info.parts += 1;
    }
    let json = serde_json::to_vec(&info).map_err(|e| Error::Corrupt(e.to_string()))?;
    store.put(&manifest_key(seq), &seal(Kind::BaseManifest, &json))?;
    Ok(info)
}
