//! A WAL segment object: consecutive commits, shipped at one moment.

use kerno_storage::format::coding::Reader;
use kerno_storage::format::wal::decode_commit;

use crate::error::{Error, Result};
use crate::object::{Cursor, Kind, seal, unseal};
use crate::wal::{Commit, format_error};

pub struct Segment {
    pub db_id: [u8; 16],
    /// When the shipper read these commits, in milliseconds since the Unix epoch.
    pub observed_ms: u64,
    pub commits: Vec<Commit>,
}

/// `wal/{first}-{last}-{observed_ms}`, zero-padded so that a listing is in sequence order.
pub fn key(first: u64, last: u64, observed_ms: u64) -> String {
    format!("wal/{first:020}-{last:020}-{observed_ms:016}")
}

/// `(first, last, observed_ms)` of a key made by [`key`].
pub fn parse_key(key: &str) -> Option<(u64, u64, u64)> {
    let mut parts = key.strip_prefix("wal/")?.split('-');
    let mut next = |width: usize| {
        let p = parts.next()?;
        (p.len() == width && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok())?
    };
    let parsed = (next(20)?, next(20)?, next(16)?);
    parts.next().is_none().then_some(parsed)
}

/// The object for `commits`, which are consecutive and not empty.
pub fn encode(db_id: [u8; 16], observed_ms: u64, commits: &[Commit]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&db_id);
    body.extend_from_slice(&commits[0].seq.to_le_bytes());
    body.extend_from_slice(&observed_ms.to_le_bytes());
    body.extend_from_slice(&(commits.len() as u32).to_le_bytes());
    let mut one = Vec::new();
    for c in commits {
        one.clear();
        c.body.encode_into(&mut one);
        body.extend_from_slice(&(one.len() as u32).to_le_bytes());
        body.extend_from_slice(&one);
    }
    seal(Kind::WalSegment, &body)
}

pub fn decode(bytes: &[u8], name: &str) -> Result<Segment> {
    let body = unseal(Kind::WalSegment, bytes, name)?;
    let mut c = Cursor::new(body, name);
    let mut db_id = [0u8; 16];
    db_id.copy_from_slice(c.take(16)?);
    let first = c.u64()?;
    let observed_ms = c.u64()?;
    let count = c.u32()?;
    let mut commits = Vec::new();
    for i in 0..u64::from(count) {
        let len = c.u32()? as usize;
        let mut reader = Reader::new(c.take(len)?, 0, "BAK-1");
        let body = decode_commit(&mut reader).map_err(|e| format_error(name, &e))?;
        if !reader.is_empty() {
            return Err(Error::Corrupt(format!("{name}: bytes left after a commit")));
        }
        commits.push(Commit { seq: first + i, body });
    }
    if !c.is_empty() {
        return Err(Error::Corrupt(format!("{name}: bytes left after the last commit")));
    }
    Ok(Segment { db_id, observed_ms, commits })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_storage::format::wal::CanonicalCommit;

    fn commit(seq: u64) -> Commit {
        Commit {
            seq,
            body: CanonicalCommit {
                ranges: vec![(b"a".to_vec(), b"b".to_vec())],
                points: vec![(b"k".to_vec(), Some(b"v".to_vec())), (b"z".to_vec(), None)],
            },
        }
    }

    #[test]
    fn a_segment_round_trips_with_its_empty_commits() {
        let empty = Commit { seq: 8, body: CanonicalCommit::default() };
        let commits = vec![commit(7), empty, commit(9)];
        let bytes = encode([3; 16], 1234, &commits);
        let seg = decode(&bytes, "s").unwrap();
        assert_eq!((seg.db_id, seg.observed_ms), ([3; 16], 1234));
        assert_eq!(seg.commits, commits);
    }

    #[test]
    fn keys_sort_by_sequence_and_parse_back() {
        let (a, b) = (key(9, 12, 5), key(10, 11, 1));
        assert!(a < b);
        assert_eq!(parse_key(&a), Some((9, 12, 5)));
        for bad in ["wal/1-2-3", "wal/x", "base/00000000000000000009", &format!("{a}-1")] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
    }
}
