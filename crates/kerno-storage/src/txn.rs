//! Read-modify-write transactions (ADR-0002, phase1-design.md §5.7).
//!
//! A [`WriteTxn`] owns the engine's write slot (`txn_gate`) for its whole
//! life, so nothing else commits between its reads and its commit: it is
//! serializable by exclusion. It reads from a registered snapshot of the
//! state it began on, overlaid with its own writes. Before `commit` it
//! touches no shared state, only its overlay: an abort, a drop and a panic in
//! the caller's code release the slot and leave no trace.
//!
//! The overlay is three views of the same writes: the final value of every
//! key written one by one (`overlay`), the ranges deleted (`ranges`, which
//! hide the snapshot's keys, not the overlay's later writes) and the writes
//! in order (`log`), which is what is committed as one batch.

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;

use kerno_core::contract::{Snapshot, Txn};
use kerno_core::{Error, Result, WriteBatch};

use crate::db::{Gate, Inner, KernoSnapshot, check_delete, check_put, check_range};
use crate::stats::bump;

/// Largest page read from the snapshot while a scan looks for rows that the
/// transaction's ranges do not hide.
const SCAN_PAGE: usize = 1024;

/// A write transaction of a [`KernoDb`](crate::KernoDb), from
/// [`begin_write`](crate::KernoDb::begin_write).
///
/// Dropping it (or calling [`abort`](Self::abort)) discards its writes,
/// consumes no sequence and releases the write slot.
pub struct WriteTxn<'db> {
    db: &'db Inner,
    /// Where reads that the transaction's own writes do not answer go.
    /// Released when `commit` begins, so that the commit does not keep the
    /// version it read alive (and the files compactions replaced meanwhile)
    /// while it waits for room, for its group, or for the `fsync`.
    snapshot: Option<KernoSnapshot>,
    /// Final state of the keys written one by one (`None` is a deletion). A
    /// range deletion removes the entries it covers.
    overlay: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    /// The non-empty ranges deleted, `[start, end)`: they hide keys of the
    /// snapshot. What the overlay holds was written after them and wins.
    ranges: Vec<(Vec<u8>, Vec<u8>)>,
    /// The writes in order, committed as they are (the engine canonicalizes
    /// them like any batch).
    log: WriteBatch,
    /// Key and value bytes of `log`, for the batch limit.
    payload: usize,
    /// `None` once `commit` took it: what tells `Drop` that the transaction
    /// did not end by an abort.
    gate: Option<Gate<'db>>,
}

impl<'db> WriteTxn<'db> {
    pub(crate) fn new(db: &'db Inner, gate: Gate<'db>, snapshot: KernoSnapshot) -> Self {
        WriteTxn {
            db,
            snapshot: Some(snapshot),
            overlay: BTreeMap::new(),
            ranges: Vec::new(),
            log: WriteBatch::new(),
            payload: 0,
            gate: Some(gate),
        }
    }

    /// The sequence of the state the transaction reads: the latest commit
    /// when it began, and still the latest while it lives.
    pub fn read_seq(&self) -> u64 {
        self.base().seq()
    }

    fn base(&self) -> &KernoSnapshot {
        self.snapshot.as_ref().expect("a transaction reads until its commit begins")
    }

    /// The value of `key` as the transaction sees it: its own last write, or
    /// absent if one of its range deletions covers the key, else the
    /// snapshot's.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if let Some(written) = self.overlay.get(key) {
            return Ok(written.clone());
        }
        if covered(&self.ranges, key) {
            return Ok(None);
        }
        self.base().get(key)
    }

    /// Up to `limit` entries with keys within the bounds as the transaction
    /// sees them, in ascending key order; paginated like
    /// [`Snapshot::scan`].
    pub fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if limit == 0 || empty_bounds(start, end) {
            return Ok(Vec::new());
        }
        let mut written = self.overlay.range::<[u8], _>((start, end)).peekable();
        let mut base = BasePages {
            snapshot: self.base(),
            ranges: &self.ranges,
            from: match start {
                Bound::Included(k) => Bound::Included(k.to_vec()),
                Bound::Excluded(k) => Bound::Excluded(k.to_vec()),
                Bound::Unbounded => Bound::Unbounded,
            },
            end,
            page: limit.clamp(16, SCAN_PAGE),
            buffered: VecDeque::new(),
            done: false,
        };
        let mut out = Vec::with_capacity(limit.min(SCAN_PAGE));
        while out.len() < limit {
            // Which source holds the next key; on a tie the overlay wins.
            let order = match (written.peek(), base.peek()?) {
                (None, None) => break,
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()),
            };
            if order == Ordering::Greater {
                out.extend(base.pop());
                continue;
            }
            if order == Ordering::Equal {
                base.pop();
            }
            if let Some((key, Some(value))) = written.next() {
                out.push((key.clone(), value.clone()));
            }
        }
        Ok(out)
    }

    /// Writes `key`. `InvalidArgument` if the key or the value is over the
    /// limits of the database, or the transaction would pass the limits of a
    /// batch; the write is then not made and the transaction stays usable.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        let limits = self.db.limits();
        check_put(key, value, limits)?;
        self.admit(key.len() + value.len())?;
        self.log.put(key, value);
        self.overlay.insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    /// Deletes `key` (see [`put`](Self::put) for the errors).
    pub fn delete(&mut self, key: &[u8]) -> Result<()> {
        check_delete(key, self.db.limits())?;
        self.admit(key.len())?;
        self.log.delete(key);
        self.overlay.insert(key.to_vec(), None);
        Ok(())
    }

    /// Deletes `[start, end)`; `start == end` deletes nothing but still makes
    /// the transaction a commit that consumes a sequence. `InvalidArgument`
    /// for `start > end` (see [`put`](Self::put) for the rest).
    pub fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()> {
        check_range(start, end, self.db.limits())?;
        self.admit(start.len() + end.len())?;
        self.log.delete_range(start, end);
        if start < end {
            self.overlay.retain(|k, _| !(start <= k.as_slice() && k.as_slice() < end));
            self.ranges.push((start.to_vec(), end.to_vec()));
        }
        Ok(())
    }

    /// Whether one more operation of `bytes` bytes of keys and values keeps
    /// the log within the batch limits (what `commit` would reject).
    fn admit(&mut self, bytes: usize) -> Result<()> {
        let limits = self.db.limits();
        if self.log.len() >= limits.max_batch_ops {
            return Err(Error::InvalidArgument(format!(
                "transaction with more than {} operations exceeds the batch limit",
                limits.max_batch_ops
            )));
        }
        if self.payload.saturating_add(bytes) > limits.max_batch_bytes {
            return Err(Error::InvalidArgument(format!(
                "transaction of over {} payload bytes exceeds the batch limit",
                limits.max_batch_bytes
            )));
        }
        self.payload += bytes;
        Ok(())
    }

    /// Commits the writes as one batch with the next sequence and returns it,
    /// with the durability of [`Engine::commit`](kerno_core::contract::Engine::commit).
    /// A transaction without writes returns the current sequence and
    /// consumes none. The write slot is released as soon as the batch is
    /// queued, before the wait for its group.
    pub fn commit(mut self) -> Result<u64> {
        let gate = self.gate.take().expect("the write slot is held until the transaction ends");
        self.snapshot = None;
        self.overlay.clear();
        self.ranges.clear();
        let seq = self.db.commit_txn(gate, &self.log)?;
        bump(&self.db.stats().txns_committed, 1);
        Ok(seq)
    }

    /// Discards the writes (same as dropping the transaction).
    pub fn abort(self) {}
}

impl Drop for WriteTxn<'_> {
    fn drop(&mut self) {
        if self.gate.is_some() {
            bump(&self.db.stats().txns_aborted, 1);
        }
    }
}

impl Txn for WriteTxn<'_> {
    fn read_seq(&self) -> u64 {
        WriteTxn::read_seq(self)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        WriteTxn::get(self, key)
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        WriteTxn::scan(self, start, end, limit)
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        WriteTxn::put(self, key, value)
    }

    fn delete(&mut self, key: &[u8]) -> Result<()> {
        WriteTxn::delete(self, key)
    }

    fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()> {
        WriteTxn::delete_range(self, start, end)
    }

    fn commit(self) -> Result<u64> {
        WriteTxn::commit(self)
    }
}

/// Whether some range of `ranges` (`[start, end)`) covers `key`.
fn covered(ranges: &[(Vec<u8>, Vec<u8>)], key: &[u8]) -> bool {
    ranges.iter().any(|(start, end)| start.as_slice() <= key && key < end.as_slice())
}

/// Whether no key can lie within the bounds: the pairs on which a scan
/// returns nothing and `BTreeMap::range` would panic.
fn empty_bounds(start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    match (start, end) {
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => {
            a > b || (a == b && !matches!((start, end), (Bound::Included(_), Bound::Included(_))))
        }
        _ => false,
    }
}

fn as_ref(b: &Bound<Vec<u8>>) -> Bound<&[u8]> {
    match b {
        Bound::Included(k) => Bound::Included(k),
        Bound::Excluded(k) => Bound::Excluded(k),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// The snapshot rows of a scan that the transaction's ranges do not hide,
/// read page by page so that a scan whose first pages are all hidden still
/// fills its limit.
struct BasePages<'a> {
    snapshot: &'a KernoSnapshot,
    ranges: &'a [(Vec<u8>, Vec<u8>)],
    from: Bound<Vec<u8>>,
    end: Bound<&'a [u8]>,
    page: usize,
    buffered: VecDeque<(Vec<u8>, Vec<u8>)>,
    done: bool,
}

impl BasePages<'_> {
    fn peek(&mut self) -> Result<Option<&(Vec<u8>, Vec<u8>)>> {
        while self.buffered.is_empty() && !self.done {
            let page = self.snapshot.scan(as_ref(&self.from), self.end, self.page)?;
            self.done = page.len() < self.page;
            if let Some((last, _)) = page.last() {
                self.from = Bound::Excluded(last.clone());
            }
            self.buffered.extend(page.into_iter().filter(|(k, _)| !covered(self.ranges, k)));
        }
        Ok(self.buffered.front())
    }

    fn pop(&mut self) -> Option<(Vec<u8>, Vec<u8>)> {
        self.buffered.pop_front()
    }
}
