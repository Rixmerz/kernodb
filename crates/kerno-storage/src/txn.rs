//! Read-modify-write transactions (ADR-0002, phase1-design.md §5.7).
//!
//! A [`WriteTxn`] owns the engine's write slot (`txn_gate`) for its whole
//! life, so nothing else commits between its reads and its commit: it is
//! serializable by exclusion. It reads from a registered snapshot of the
//! state it began on, overlaid with its own writes. Before `commit` it
//! touches no shared state, only its overlay: an abort, a drop and a panic in
//! the caller's code release the slot and leave no trace.
//!
//! The overlay is two views of the same writes: the keys written one by one
//! (`overlay`) and the ranges deleted (`ranges`, which hide the snapshot's
//! keys, not the overlay's later writes). Together they are the canonical
//! form of the writes (WAL-C1): the ranges, then the points in key order,
//! each key once. Writes are appended to a buffer in call order; it is sorted
//! once at commit (last write of a key wins). A point read finds the last
//! write of its key through a hash index of the buffer; only a scan, which
//! needs the sorted view, folds the buffer into a map first. So a batch that
//! writes and reads points pays one sort and no per-record tree insert.

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;
use std::sync::{Mutex, MutexGuard, PoisonError};

use kerno_core::contract::{Snapshot, Txn};
use kerno_core::{Error, Result};

use crate::db::{Gate, Inner, KernoSnapshot, check_delete, check_put, check_range};
use crate::format::wal::CanonicalCommit;
use crate::stats::bump;

/// Largest page read from the snapshot while a scan looks for rows that the
/// transaction's ranges do not hide.
const SCAN_PAGE: usize = 1024;

type Entry = (Vec<u8>, Option<Vec<u8>>);

/// The point writes of a transaction (`None` is a deletion).
#[derive(Default)]
struct Overlay {
    /// Final state of the keys already folded in, in key order.
    map: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    /// Writes since the last fold, in call order: a later entry of a key
    /// replaces an earlier one, and any entry replaces the map's.
    pending: Vec<Entry>,
    /// Hash index of `pending` for `get`: open addressing, each slot `0`
    /// (empty) or `1 +` the position in `pending` of the **last** write of a
    /// key. A point read between writes is then one probe instead of a fold
    /// of the buffer into the ordered map (a tree insert per write) or a scan
    /// of the buffer. It is an index only: `pending` stays the truth, so the
    /// canonical form (`into_sorted`, `fold`) does not depend on it. Length
    /// a power of two, at most half full; empty until the first write.
    slots: Vec<u32>,
    /// Positions in `pending` of its lowest and its highest key (meaningless
    /// while it is empty): a scan whose bounds lie wholly outside them needs
    /// nothing from the buffer, so it does not fold it.
    low: usize,
    high: usize,
}

/// FNV-1a folded over eight bytes at a time: the keys are short (tens of
/// bytes) and differ mostly in their last bytes, and the table only needs a
/// spread, not resistance to chosen keys (a transaction hashes its own writes).
fn hash_key(key: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut h = (key.len() as u64).wrapping_mul(K);
    let (words, rest) = key.as_chunks::<8>();
    for w in words {
        h = (h.rotate_left(27) ^ u64::from_le_bytes(*w)).wrapping_mul(K);
    }
    let mut tail = [0u8; 8];
    tail[..rest.len()].copy_from_slice(rest);
    h = (h.rotate_left(27) ^ u64::from_le_bytes(tail)).wrapping_mul(K);
    h ^ (h >> 29)
}

impl Overlay {
    /// The slot of `key`: the one that holds its last write, or the empty
    /// one where it goes. The table is never full (`index` keeps it at most
    /// half full), so the probe ends.
    fn slot(&self, key: &[u8]) -> usize {
        let mask = self.slots.len() - 1;
        let mut at = hash_key(key) as usize & mask;
        loop {
            match self.slots[at] {
                0 => return at,
                n if self.pending[n as usize - 1].0 == key => return at,
                _ => at = (at + 1) & mask,
            }
        }
    }

    /// Points the index at the last entry of `pending`, just pushed.
    fn index_last(&mut self) {
        if self.pending.len() * 2 > self.slots.len() {
            return self.reindex();
        }
        let at = self.slot(&self.pending[self.pending.len() - 1].0);
        self.slots[at] = self.pending.len() as u32;
    }

    /// Rebuilds the index from `pending`, with room to double.
    fn reindex(&mut self) {
        let len = (self.pending.len() * 4).next_power_of_two().max(64);
        self.slots.clear();
        self.slots.resize(len, 0);
        // In call order, so the slot of a key ends at its last write.
        for i in 0..self.pending.len() {
            let at = self.slot(&self.pending[i].0);
            self.slots[at] = i as u32 + 1;
        }
    }

    fn push(&mut self, entry: Entry) {
        let at = self.pending.len();
        if at == 0 {
            (self.low, self.high) = (0, 0);
        } else if entry.0 > self.pending[self.high].0 {
            self.high = at;
        } else if entry.0 < self.pending[self.low].0 {
            self.low = at;
        }
        self.pending.push(entry);
        self.index_last();
    }

    /// Finds the lowest and highest key of `pending` again, after entries were removed.
    fn rebound(&mut self) {
        (self.low, self.high) = (0, 0);
        for (i, (k, _)) in self.pending.iter().enumerate() {
            if *k > self.pending[self.high].0 {
                self.high = i;
            } else if *k < self.pending[self.low].0 {
                self.low = i;
            }
        }
    }

    /// Whether a buffered write may lie within the bounds: `false` only when
    /// every buffered key is below `start` or above `end`.
    fn pending_within(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let (low, high) = (self.pending[self.low].0.as_slice(), self.pending[self.high].0.as_slice());
        let above = match start {
            Bound::Included(s) => s > high,
            Bound::Excluded(s) => s >= high,
            Bound::Unbounded => false,
        };
        let below = match end {
            Bound::Included(e) => e < low,
            Bound::Excluded(e) => e <= low,
            Bound::Unbounded => false,
        };
        !(above || below)
    }

    /// The last write of `key`, if the transaction wrote it.
    fn get(&self, key: &[u8]) -> Option<&Option<Vec<u8>>> {
        if !self.pending.is_empty() {
            match self.slots[self.slot(key)] {
                0 => {}
                n => return Some(&self.pending[n as usize - 1].1),
            }
        }
        self.map.get(key)
    }

    fn fold(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        for (k, v) in self.pending.drain(..) {
            self.map.insert(k, v);
        }
        self.slots.clear();
    }

    /// The points in key order, each key once, the last write of it.
    fn into_sorted(self) -> Vec<Entry> {
        // Call order inside a run of one key, as a stable sort keeps it:
        // the run's last entry is the write that stands.
        let mut buffered = sort_writes(self.pending);
        // In place: of a run of one key the last write is moved into the
        // slot that stays.
        buffered.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                std::mem::swap(later, kept);
            }
            same
        });
        if self.map.is_empty() {
            return buffered;
        }
        // Two sorted runs, merged: what a scan folded into the map earlier,
        // and the buffer, whose writes are later and win on a key of both.
        // Folding the buffer into the map instead costs a tree insert per
        // write.
        let mut out: Vec<Entry> = Vec::with_capacity(self.map.len() + buffered.len());
        let mut folded = self.map.into_iter().peekable();
        for e in buffered {
            while let Some(f) = folded.next_if(|f| f.0 < e.0) {
                out.push(f);
            }
            folded.next_if(|f| f.0 == e.0);
            out.push(e);
        }
        out.extend(folded);
        out
    }
}

/// The first 16 bytes of `key` as a big-endian number, zero-padded: ordered
/// like the keys wherever they differ (a key that ends first reads as zeros,
/// and is the lower one unless the other continues with zeros, a tie).
fn key_prefix(key: &[u8]) -> u128 {
    let mut b = [0u8; 16];
    let n = key.len().min(16);
    b[..n].copy_from_slice(&key[..n]);
    u128::from_be_bytes(b)
}

/// `writes` in key order, writes of one key in call order: what a stable
/// sort by key gives. The sort runs over small `(prefix, position)` items, so
/// that most comparisons are of two numbers; whole keys are compared only
/// where 16 bytes tie. The sort is stable, so equal keys keep their order.
fn sort_writes(writes: Vec<Entry>) -> Vec<Entry> {
    let mut order: Vec<(u128, usize)> = writes.iter().enumerate().map(|(i, (k, _))| (key_prefix(k), i)).collect();
    order.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| writes[a.1].0.cmp(&writes[b.1].0)));
    let mut writes = writes;
    order.into_iter().map(|(_, i)| std::mem::take(&mut writes[i])).collect()
}

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
    /// The keys written one by one. A range deletion removes the entries it
    /// covers. A mutex only so that `get` and `scan` (`&self`) can fold.
    overlay: Mutex<Overlay>,
    /// The non-empty ranges deleted, `[start, end)`: they hide keys of the
    /// snapshot. What the overlay holds was written after them and wins.
    ranges: Vec<(Vec<u8>, Vec<u8>)>,
    /// How many writes were made, for the batch limit. A write that replaces
    /// an earlier one of the same key still counts, as it did in a batch.
    ops: usize,
    /// Key and value bytes of those writes, for the batch limit.
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
            overlay: Mutex::new(Overlay::default()),
            ranges: Vec::new(),
            ops: 0,
            payload: 0,
            gate: Some(gate),
        }
    }

    /// The sequence of the state the transaction reads: the latest commit
    /// when it began, and still the latest while it lives.
    pub fn read_seq(&self) -> u64 {
        self.base().seq()
    }

    fn overlay(&self) -> MutexGuard<'_, Overlay> {
        self.overlay.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn base(&self) -> &KernoSnapshot {
        self.snapshot.as_ref().expect("a transaction reads until its commit begins")
    }

    /// The value of `key` as the transaction sees it: its own last write, or
    /// absent if one of its range deletions covers the key, else the
    /// snapshot's.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        {
            if let Some(written) = self.overlay().get(key) {
                return Ok(written.clone());
            }
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
        let mut ov = self.overlay();
        // The buffer is folded into the ordered map only when one of its
        // writes can be in the range: a caller that writes its keys in
        // ascending order and seeks ahead of them (an index flush) then
        // never pays a tree insert per write. What the map already holds is
        // merged below either way.
        if ov.pending_within(start, end) {
            ov.fold();
        }
        let mut written = ov.map.range::<[u8], _>((start, end)).peekable();
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
        self.put_owned(key.to_vec(), value.to_vec())
    }

    /// [`put`](Self::put) that keeps the caller's buffers instead of copying them.
    pub fn put_owned(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        check_put(&key, &value, self.db.limits())?;
        self.admit(key.len() + value.len())?;
        self.overlay.get_mut().unwrap_or_else(PoisonError::into_inner).push((key, Some(value)));
        Ok(())
    }

    /// Deletes `key` (see [`put`](Self::put) for the errors).
    pub fn delete(&mut self, key: &[u8]) -> Result<()> {
        check_delete(key, self.db.limits())?;
        self.admit(key.len())?;
        self.overlay.get_mut().unwrap_or_else(PoisonError::into_inner).push((key.to_vec(), None));
        Ok(())
    }

    /// Deletes `[start, end)`; `start == end` deletes nothing but still makes
    /// the transaction a commit that consumes a sequence. `InvalidArgument`
    /// for `start > end` (see [`put`](Self::put) for the rest).
    pub fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()> {
        check_range(start, end, self.db.limits())?;
        self.admit(start.len() + end.len())?;
        if start < end {
            let ov = self.overlay.get_mut().unwrap_or_else(PoisonError::into_inner);
            let hit = |k: &[u8]| start <= k && k < end;
            ov.map.retain(|k, _| !hit(k));
            ov.pending.retain(|(k, _)| !hit(k));
            ov.reindex();
            ov.rebound();
            self.ranges.push((start.to_vec(), end.to_vec()));
        }
        Ok(())
    }

    /// Whether one more operation of `bytes` bytes of keys and values keeps
    /// the log within the batch limits (what `commit` would reject).
    fn admit(&mut self, bytes: usize) -> Result<()> {
        let limits = self.db.limits();
        if self.ops >= limits.max_batch_ops {
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
        self.ops += 1;
        Ok(())
    }

    /// The writes in canonical form (WAL-C1), taken out of the transaction:
    /// what [`canonicalize`](crate::format::wal::canonicalize) gives for the
    /// same writes in order: range deletions already removed the entries
    /// written before them, and the last write of a key wins.
    fn take_canonical(&mut self) -> CanonicalCommit<Vec<u8>> {
        let mut ranges = std::mem::take(&mut self.ranges);
        ranges.sort_unstable();
        ranges.dedup();
        CanonicalCommit {
            ranges,
            points: std::mem::take(self.overlay.get_mut().unwrap_or_else(PoisonError::into_inner)).into_sorted(),
        }
    }

    /// Commits the writes as one batch with the next sequence and returns it,
    /// with the durability of [`Engine::commit`](kerno_core::contract::Engine::commit).
    /// A transaction without writes returns the current sequence and
    /// consumes none. The write slot is released as soon as the batch is
    /// queued, before the wait for its group.
    pub fn commit(mut self) -> Result<u64> {
        let gate = self.gate.take().expect("the write slot is held until the transaction ends");
        self.snapshot = None;
        let commit = self.take_canonical();
        let seq = self.db.commit_txn(gate, commit, self.ops > 0, false)?;
        bump(&self.db.stats().txns_committed, 1);
        Ok(seq)
    }

    /// Like [`commit`](Self::commit), but the batch asks for no `fsync` of
    /// its own, even with [`Durability::Sync`](kerno_core::Durability::Sync)
    /// (decision 12c of `stage3-design.md`): it is visible and acknowledged
    /// at once, and becomes durable with the next `fsync` of the log, that
    /// of any later commit that is not deferred, of [`KernoDb::sync`] or of
    /// the log rotation that a full write buffer or `close` causes. The log
    /// is written in commit order and recovered as a prefix, so a power loss
    /// can only take back a tail of deferred commits; a later durable
    /// commit never survives without the deferred commits before it.
    ///
    /// [`KernoDb::sync`]: crate::KernoDb::sync
    pub fn commit_deferred(mut self) -> Result<u64> {
        let gate = self.gate.take().expect("the write slot is held until the transaction ends");
        self.snapshot = None;
        let commit = self.take_canonical();
        let seq = self.db.commit_txn(gate, commit, self.ops > 0, true)?;
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

    fn put_owned(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        WriteTxn::put_owned(self, key, value)
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

    fn commit_deferred(self) -> Result<(u64, bool)> {
        WriteTxn::commit_deferred(self).map(|seq| (seq, false))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u64) -> Vec<u8> {
        format!("key-{n:04}").into_bytes()
    }

    /// The overlay against a plain ordered map, through growth of the hash
    /// index, repeated keys, folds and removals: `get` and the canonical
    /// form must agree with the map after every step.
    #[test]
    fn overlay_matches_an_ordered_map() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..20 {
            let (mut ov, mut model) = (Overlay::default(), BTreeMap::new());
            for step in 0..400 {
                let (k, r) = (key(next() % 150), next() % 100);
                match r {
                    // A fold, as a scan over the buffered keys does.
                    0..=2 => ov.fold(),
                    // A range deletion removes the entries it covers from both views.
                    3 => {
                        let end = key(next() % 150);
                        let (lo, hi) = if k <= end { (k.clone(), end) } else { (end, k.clone()) };
                        ov.map.retain(|x, _| !(lo <= *x && *x < hi));
                        ov.pending.retain(|(x, _)| !(lo <= *x && *x < hi));
                        ov.reindex();
                        ov.rebound();
                        model.retain(|x: &Vec<u8>, _| !(lo <= *x && *x < hi));
                    }
                    4..=20 => {
                        ov.push((k.clone(), None));
                        model.insert(k.clone(), None);
                    }
                    _ => {
                        let v = Some(vec![round as u8, step as u8]);
                        ov.push((k.clone(), v.clone()));
                        model.insert(k.clone(), v);
                    }
                }
                let probe = key(next() % 150);
                assert_eq!(ov.get(&probe), model.get(&probe), "round {round} step {step}");
                if let (Some((low, _)), Some((high, _))) = (ov.pending.get(ov.low), ov.pending.get(ov.high)) {
                    assert!(ov.pending.iter().all(|(x, _)| low <= x && x <= high));
                }
            }
            let sorted = ov.into_sorted();
            assert_eq!(sorted, model.into_iter().collect::<Vec<_>>(), "round {round}");
        }
    }

    /// `sort_writes` orders like a stable sort by key, on keys that end early, hold NULs, tie
    /// on their first 16 bytes and repeat.
    #[test]
    fn sort_writes_is_a_stable_sort_by_key() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..50 {
            let writes: Vec<Entry> = (0..300u32)
                .map(|i| {
                    let len = (next() % 24) as usize;
                    // Few distinct bytes, so keys share long prefixes and repeat.
                    let key = (0..len).map(|_| [0u8, 1, 0xff][(next() % 3) as usize]).collect();
                    (key, Some(i.to_be_bytes().to_vec()))
                })
                .collect();
            let mut expected = writes.clone();
            expected.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(sort_writes(writes), expected, "round {round}");
        }
    }

    /// A scan folds the buffer only when one of its writes can lie within the bounds.
    #[test]
    fn pending_within_tells_when_a_scan_needs_the_buffer() {
        let mut ov = Overlay::default();
        let all = (Bound::Unbounded, Bound::Unbounded);
        assert!(!ov.pending_within(all.0, all.1), "nothing buffered");
        for n in [20, 10, 30] {
            ov.push((key(n), None));
        }
        let (k5, k10, k30, k40) = (key(5), key(10), key(30), key(40));
        assert!(ov.pending_within(all.0, all.1));
        assert!(!ov.pending_within(Bound::Included(&k40), Bound::Unbounded), "above the highest");
        assert!(!ov.pending_within(Bound::Excluded(&k30), Bound::Unbounded), "after the highest");
        assert!(ov.pending_within(Bound::Included(&k30), Bound::Unbounded), "the highest itself");
        assert!(!ov.pending_within(Bound::Unbounded, Bound::Excluded(&k10)), "before the lowest");
        assert!(ov.pending_within(Bound::Unbounded, Bound::Included(&k10)), "the lowest itself");
        assert!(!ov.pending_within(Bound::Unbounded, Bound::Included(&k5)), "below the lowest");
        assert!(ov.pending_within(Bound::Included(&k5), Bound::Excluded(&k40)));
    }
}
