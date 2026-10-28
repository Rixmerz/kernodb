//! In-memory write buffer: a lock-free skip list keyed by
//! (user key ascending, sequence descending), holding values and tombstones.
//!
//! The single writer inserts while any number of readers iterate; a reader
//! at snapshot `seq` ignores entries with a larger sequence number, so the
//! writer never has to copy or lock the table.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::ops::Bound;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

use crossbeam_skiplist::SkipMap;
use crossbeam_skiplist::map::Entry;
use kerno_core::{Op, Result};

use crate::iter::InternalIterator;

/// Rough per-entry bookkeeping cost (skip-list node, key struct, vectors).
const ENTRY_OVERHEAD: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternalKey {
    pub user_key: Vec<u8>,
    pub seq: u64,
}

impl Ord for InternalKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.user_key.cmp(&other.user_key).then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for InternalKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `None` is a tombstone.
type Value = Option<Vec<u8>>;

pub struct Memtable {
    map: SkipMap<InternalKey, Value>,
    bytes: AtomicUsize,
    max_seq: AtomicU64,
    /// The WAL file that holds this memtable's commits.
    wal_number: u64,
}

impl Memtable {
    pub fn new(wal_number: u64) -> Self {
        Memtable { map: SkipMap::new(), bytes: AtomicUsize::new(0), max_seq: AtomicU64::new(0), wal_number }
    }

    pub fn wal_number(&self) -> u64 {
        self.wal_number
    }

    pub fn approx_bytes(&self) -> usize {
        self.bytes.load(Relaxed)
    }

    /// Largest sequence number applied (0 if empty).
    pub fn max_seq(&self) -> u64 {
        self.max_seq.load(Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Applies one batch under sequence `seq`. Later operations on the same
    /// key win, so only the last operation per key is inserted.
    pub fn apply(&self, seq: u64, ops: &[Op]) {
        if ops.len() == 1 {
            self.insert(seq, &ops[0]);
        } else {
            let mut seen: HashSet<&[u8]> = HashSet::with_capacity(ops.len());
            for op in ops.iter().rev() {
                if seen.insert(op.key()) {
                    self.insert(seq, op);
                }
            }
        }
        self.max_seq.fetch_max(seq, Relaxed);
    }

    fn insert(&self, seq: u64, op: &Op) {
        let (key, value) = match op {
            Op::Put { key, value } => (key, Some(value.clone())),
            Op::Delete { key } => (key, None),
            Op::DeleteRange { .. } => unreachable!("delete_range is rejected at the entry of commit"),
        };
        let size = key.len() + value.as_ref().map_or(0, Vec::len) + ENTRY_OVERHEAD;
        self.bytes.fetch_add(size, Relaxed);
        self.map.insert(InternalKey { user_key: key.clone(), seq }, value);
    }

    /// Newest version of `probe.user_key` with sequence `<= probe.seq`:
    /// `Some(Some(v))` value, `Some(None)` tombstone, `None` not present.
    pub fn get(&self, probe: &InternalKey) -> Option<Value> {
        let e = self.map.lower_bound(Bound::Included(probe))?;
        (e.key().user_key == probe.user_key).then(|| e.value().clone())
    }

    /// Iterator over every version visible at `seq` (several per key,
    /// newest first).
    pub fn iter(&self, seq: u64) -> MemIter<'_> {
        MemIter { map: &self.map, cur: None, seq }
    }
}

pub struct MemIter<'a> {
    map: &'a SkipMap<InternalKey, Value>,
    cur: Option<Entry<'a, InternalKey, Value>>,
    seq: u64,
}

impl<'a> MemIter<'a> {
    fn skip_invisible(&mut self) {
        while let Some(e) = &self.cur {
            if e.key().seq <= self.seq {
                break;
            }
            let next = e.next();
            self.cur = next;
        }
    }

    fn entry(&self) -> &Entry<'a, InternalKey, Value> {
        self.cur.as_ref().expect("iterator is not valid")
    }
}

impl InternalIterator for MemIter<'_> {
    fn valid(&self) -> bool {
        self.cur.is_some()
    }

    fn seek_to_first(&mut self) -> Result<()> {
        self.cur = self.map.front();
        self.skip_invisible();
        Ok(())
    }

    fn seek(&mut self, target: &[u8]) -> Result<()> {
        let probe = InternalKey { user_key: target.to_vec(), seq: u64::MAX };
        self.cur = self.map.lower_bound(Bound::Included(&probe));
        self.skip_invisible();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        let next = self.entry().next();
        self.cur = next;
        self.skip_invisible();
        Ok(())
    }

    fn key(&self) -> &[u8] {
        &self.entry().key().user_key
    }

    fn seq(&self) -> u64 {
        self.entry().key().seq
    }

    fn value(&self) -> Option<&[u8]> {
        self.entry().value().as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(k: &str, v: &str) -> Op {
        Op::Put { key: k.into(), value: v.into() }
    }

    fn del(k: &str) -> Op {
        Op::Delete { key: k.into() }
    }

    fn probe(k: &str, seq: u64) -> InternalKey {
        InternalKey { user_key: k.into(), seq }
    }

    #[test]
    fn versions_are_filtered_by_sequence() {
        let m = Memtable::new(1);
        m.apply(1, &[put("a", "1"), put("b", "1")]);
        m.apply(2, &[put("a", "2"), del("b")]);
        m.apply(3, &[put("a", "x"), del("a"), put("a", "3")]);
        assert_eq!(m.get(&probe("a", 0)), None);
        assert_eq!(m.get(&probe("a", 1)), Some(Some(b"1".to_vec())));
        assert_eq!(m.get(&probe("a", 2)), Some(Some(b"2".to_vec())));
        assert_eq!(m.get(&probe("a", 3)), Some(Some(b"3".to_vec())));
        assert_eq!(m.get(&probe("b", 1)), Some(Some(b"1".to_vec())));
        assert_eq!(m.get(&probe("b", 9)), Some(None));
        assert_eq!(m.get(&probe("c", 9)), None);
        assert_eq!(m.max_seq(), 3);

        let mut it = m.iter(2);
        it.seek_to_first().unwrap();
        let mut seen = Vec::new();
        while it.valid() {
            seen.push((it.key().to_vec(), it.seq(), it.value().map(<[u8]>::to_vec)));
            it.next().unwrap();
        }
        assert_eq!(
            seen,
            vec![
                (b"a".to_vec(), 2, Some(b"2".to_vec())),
                (b"a".to_vec(), 1, Some(b"1".to_vec())),
                (b"b".to_vec(), 2, None),
                (b"b".to_vec(), 1, Some(b"1".to_vec())),
            ]
        );
        let mut it = m.iter(3);
        it.seek(b"a\0").unwrap();
        assert_eq!((it.key(), it.seq()), (&b"b"[..], 2));
    }
}
