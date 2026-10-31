//! `Emulated<E>`: range deletions and write transactions for any engine,
//! emulated in the test kit.
//!
//! Engines that do not implement [`Op::DeleteRange`] (the frozen spikes and
//! the production engines of the benchmark) reject a batch that contains
//! one. To run the range deletion suite and the differential test on them
//! anyway, [`Emulated<E>`] wraps the engine and rewrites every batch with a
//! range deletion before committing it: under an internal commit mutex it
//! takes a snapshot of the latest state, and each range `[start, end)`
//! becomes, at its position in the batch, one point deletion per key that
//! exists at that point (keys of the snapshot in the range, plus keys put
//! earlier in the same batch). The rewritten batch is applied atomically by
//! the wrapped engine, so the result is the contract's semantics: operations
//! in batch order, all with one commit sequence.
//!
//! The emulation is a test oracle, not an implementation: its cost grows
//! with the number of live keys a range covers, a range over many keys
//! becomes a batch that large, and nothing of the range is stored, so it
//! says nothing about how an engine stores range tombstones. Every commit,
//! with or without ranges, takes the commit mutex, so a commit from another
//! thread can never land between the snapshot and the rewritten commit.
//! The rewrite never introduces the empty key, which some engines reject
//! (fjall panics on it): it only deletes it when the batch's own operations
//! or the stored state already contain it, which an engine that rejects it
//! can never have.
//!
//! # Transactions
//!
//! `Emulated<E>` also implements [`TxnEngine`]: [`begin_write`] takes the
//! commit mutex and keeps it for the whole life of the transaction (so
//! transactions are exclusive and blind commits wait for them, as the
//! contract requires), reads from a snapshot of the latest state overlaid
//! with the transaction's own writes ([`EmulatedTxn`]), and commits the
//! writes as one batch through the same path as [`Engine::commit`], range
//! deletions rewritten included. Nothing reaches the wrapped engine before
//! the commit, so an abort, a drop or a panic leaves no trace in it.
//!
//! [`begin_write`]: TxnEngine::begin_write

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Engine, Options, RangeDeleteEngine, Snapshot, Txn, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_core::{MAX_KEY_LEN, Op, Result, WriteBatch};
use parking_lot::{Mutex, MutexGuard};

/// Page size of the snapshot scans that list the keys of a range.
const SCAN_PAGE: usize = 1024;

/// `E` with range deletions emulated by point deletions (see the
/// [module documentation](self)).
pub struct Emulated<E> {
    inner: E,
    commit: Mutex<()>,
}

impl<E: Engine> Emulated<E> {
    /// The wrapped engine.
    pub fn inner(&self) -> &E {
        &self.inner
    }

    /// Commits `batch` with the commit mutex held by the caller.
    fn commit_locked(&self, batch: &WriteBatch) -> Result<u64> {
        if !batch.has_range_deletes() {
            return self.inner.commit(batch);
        }
        // Rejected batches must not touch the engine at all (not even a
        // snapshot): same errors as a native engine, before anything else.
        batch.validate()?;
        let rewritten = self.rewrite(batch)?;
        self.inner.commit(&rewritten)
    }

    /// `batch` with every range deletion replaced by point deletions of the
    /// keys it covers at its position in the batch. Called with the commit
    /// mutex held, after `batch.validate()`.
    fn rewrite(&self, batch: &WriteBatch) -> Result<WriteBatch> {
        let mut snap: Option<E::Snapshot> = None;
        let mut out = WriteBatch::new();
        // Keys of the point operations emitted so far.
        let mut touched: BTreeSet<Vec<u8>> = BTreeSet::new();
        for op in batch.ops() {
            match op {
                Op::Put { key, value } => {
                    out.put(key.clone(), value.clone());
                    touched.insert(key.clone());
                }
                Op::Delete { key } => {
                    out.delete(key.clone());
                    touched.insert(key.clone());
                }
                // Empty range: no effect.
                Op::DeleteRange { start, end } if start >= end => {}
                Op::DeleteRange { start, end } => {
                    let range = (Bound::Included(start.as_slice()), Bound::Excluded(end.as_slice()));
                    let mut doomed: BTreeSet<Vec<u8>> = touched.range::<[u8], _>(range).cloned().collect();
                    // `start` is in the range, so deleting it is part of the
                    // range's effect whether or not it exists; it also keeps
                    // the rewritten batch non-empty, so that it still gets a
                    // commit sequence. Not the empty key, which some engines
                    // reject (fjall panics on it): if it exists, the scan
                    // below finds it, and a rewrite left empty falls back to
                    // [`no_op`].
                    if !start.is_empty() {
                        doomed.insert(start.clone());
                    }
                    if snap.is_none() {
                        snap = Some(self.inner.snapshot()?);
                    }
                    let snap = snap.as_ref().expect("taken above");
                    let mut from = Bound::Included(start.clone());
                    loop {
                        let page = snap.scan(as_ref(&from), Bound::Excluded(end.as_slice()), SCAN_PAGE)?;
                        let done = page.len() < SCAN_PAGE;
                        if let Some((last, _)) = page.last() {
                            from = Bound::Excluded(last.clone());
                        }
                        doomed.extend(page.into_iter().map(|(k, _)| k));
                        if done {
                            break;
                        }
                    }
                    for key in doomed {
                        out.delete(key.clone());
                        touched.insert(key);
                    }
                }
            }
        }
        if out.is_empty() && !batch.is_empty() {
            // Only empty ranges: a non-empty batch is still a commit and
            // consumes a sequence without changing the state. Delete a key
            // that does not exist (or rewrite an existing value as it is).
            let snap = match snap {
                Some(s) => s,
                None => self.inner.snapshot()?,
            };
            out = no_op(&snap)?;
        }
        Ok(out)
    }
}

fn as_ref(b: &Bound<Vec<u8>>) -> Bound<&[u8]> {
    match b {
        Bound::Included(k) => Bound::Included(k),
        Bound::Excluded(k) => Bound::Excluded(k),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// A batch that changes nothing in `snap`'s state: the deletion of the
/// first absent key among `"\0"`, `"\0\0"`, …, or, if all of those that
/// fit in [`MAX_KEY_LEN`] exist, a put of `"\0"` with its value. Never the
/// empty key, which some engines reject (fjall panics on it).
fn no_op<S: Snapshot>(snap: &S) -> Result<WriteBatch> {
    let mut b = WriteBatch::new();
    for len in 1..=MAX_KEY_LEN {
        let key = vec![0u8; len];
        if snap.get(&key)?.is_none() {
            b.delete(key);
            return Ok(b);
        }
    }
    let value = snap.get(&[0])?.expect("probed above: it exists");
    b.put(vec![0u8], value);
    Ok(b)
}

/// `"{inner}-emulated"`, allocated once per inner name (`Engine::name`
/// returns a `&'static str`).
fn emulated_name(inner: &'static str) -> &'static str {
    static NAMES: Mutex<BTreeMap<&'static str, &'static str>> = Mutex::new(BTreeMap::new());
    NAMES.lock().entry(inner).or_insert_with(|| Box::leak(format!("{inner}-emulated").into_boxed_str()))
}

impl<E: Engine> Engine for Emulated<E> {
    type Snapshot = E::Snapshot;

    fn name() -> &'static str {
        emulated_name(E::name())
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        Ok(Emulated { inner: E::open(vfs, dir, opts)?, commit: Mutex::new(()) })
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        let _commit = self.commit.lock();
        self.commit_locked(batch)
    }

    fn snapshot(&self) -> Result<E::Snapshot> {
        self.inner.snapshot()
    }

    fn close(self) -> Result<()> {
        self.inner.close()
    }

    fn stats(&self) -> Vec<(String, u64)> {
        self.inner.stats()
    }
}

impl<E: Engine> RangeDeleteEngine for Emulated<E> {}

impl<E: Engine> TxnEngine for Emulated<E> {
    type Txn<'a>
        = EmulatedTxn<'a, E>
    where
        Self: 'a;

    fn begin_write(&self) -> Result<EmulatedTxn<'_, E>> {
        let gate = self.commit.lock();
        let snap = self.inner.snapshot()?;
        Ok(EmulatedTxn {
            db: self,
            snap,
            overlay: BTreeMap::new(),
            ranges: Vec::new(),
            batch: WriteBatch::new(),
            _gate: gate,
        })
    }
}

/// A write transaction of [`Emulated<E>`] (see the [module
/// documentation](self)).
pub struct EmulatedTxn<'a, E: Engine> {
    db: &'a Emulated<E>,
    /// The latest state when the transaction began: nobody can commit while
    /// the commit mutex is held, so it stays the latest.
    snap: E::Snapshot,
    /// Final state of the keys the transaction wrote one by one (`None` is a
    /// deletion). A range deletion removes the entries it covers.
    overlay: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    /// The non-empty ranges deleted, `[start, end)`. Hide the snapshot's keys
    /// they cover; the overlay, which holds only what was written after them,
    /// wins over them.
    ranges: Vec<(Vec<u8>, Vec<u8>)>,
    /// The writes in order, committed as they are.
    batch: WriteBatch,
    /// Released when the transaction ends, after the commit.
    _gate: MutexGuard<'a, ()>,
}

/// Whether some range of `ranges` (`[start, end)`) covers `key`.
fn covered(ranges: &[(Vec<u8>, Vec<u8>)], key: &[u8]) -> bool {
    ranges.iter().any(|(start, end)| start.as_slice() <= key && key < end.as_slice())
}

/// Whether no key can lie within the bounds: the pairs on which
/// `Snapshot::scan` returns nothing and `BTreeMap::range` would panic.
fn empty_bounds(start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    match (start, end) {
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => {
            a > b || (a == b && !matches!((start, end), (Bound::Included(_), Bound::Included(_))))
        }
        _ => false,
    }
}

/// The snapshot entries of a scan that the transaction's ranges do not hide,
/// read page by page so that a scan whose first pages are all hidden still
/// fills its limit.
struct BasePages<'a, S: Snapshot> {
    snap: &'a S,
    ranges: &'a [(Vec<u8>, Vec<u8>)],
    from: Bound<Vec<u8>>,
    end: Bound<&'a [u8]>,
    page: usize,
    buffered: VecDeque<(Vec<u8>, Vec<u8>)>,
    done: bool,
}

impl<S: Snapshot> BasePages<'_, S> {
    fn peek(&mut self) -> Result<Option<&(Vec<u8>, Vec<u8>)>> {
        while self.buffered.is_empty() && !self.done {
            let page = self.snap.scan(as_ref(&self.from), self.end, self.page)?;
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

impl<E: Engine> Txn for EmulatedTxn<'_, E> {
    fn read_seq(&self) -> u64 {
        self.snap.seq()
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if let Some(written) = self.overlay.get(key) {
            return Ok(written.clone());
        }
        if covered(&self.ranges, key) {
            return Ok(None);
        }
        self.snap.get(key)
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if limit == 0 || empty_bounds(start, end) {
            return Ok(Vec::new());
        }
        let mut written = self.overlay.range::<[u8], _>((start, end)).peekable();
        let mut base = BasePages {
            snap: &self.snap,
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

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        WriteBatch::new().put(key, value).validate()?;
        self.batch.put(key, value);
        self.overlay.insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<()> {
        WriteBatch::new().delete(key).validate()?;
        self.batch.delete(key);
        self.overlay.insert(key.to_vec(), None);
        Ok(())
    }

    fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()> {
        WriteBatch::new().delete_range(start, end).validate()?;
        self.batch.delete_range(start, end);
        if start < end {
            self.overlay.retain(|k, _| !(start <= k.as_slice() && k.as_slice() < end));
            self.ranges.push((start.to_vec(), end.to_vec()));
        }
        Ok(())
    }

    fn commit(self) -> Result<u64> {
        if self.batch.is_empty() {
            return Ok(self.snap.seq());
        }
        self.db.commit_locked(&self.batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{Model, apply, read_all};
    use crate::reference::LogEngine;
    use crate::vfs::SimVfs;
    use kerno_core::Error;

    /// A `LogEngine` that rejects range deletions, like the spikes, and
    /// records every batch it commits.
    struct PointsOnly {
        db: LogEngine,
        log: Mutex<Vec<WriteBatch>>,
    }

    impl Engine for PointsOnly {
        type Snapshot = <LogEngine as Engine>::Snapshot;

        fn name() -> &'static str {
            "points"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            Ok(PointsOnly { db: LogEngine::open(vfs, dir, opts)?, log: Mutex::new(Vec::new()) })
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            if batch.has_range_deletes() {
                return Err(Error::InvalidArgument("delete_range not supported".into()));
            }
            self.log.lock().push(batch.clone());
            self.db.commit(batch)
        }

        fn snapshot(&self) -> Result<Self::Snapshot> {
            self.db.snapshot()
        }

        fn close(self) -> Result<()> {
            self.db.close()
        }
    }

    /// Like fjall, panics on an operation on the empty key.
    struct NoEmptyKey(PointsOnly);

    impl Engine for NoEmptyKey {
        type Snapshot = <LogEngine as Engine>::Snapshot;

        fn name() -> &'static str {
            "no-empty-key"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            Ok(NoEmptyKey(PointsOnly::open(vfs, dir, opts)?))
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            for op in batch.ops() {
                assert!(!op.key().is_empty(), "the empty key reached the wrapped engine: {batch:?}");
            }
            self.0.commit(batch)
        }

        fn snapshot(&self) -> Result<Self::Snapshot> {
            self.0.snapshot()
        }

        fn close(self) -> Result<()> {
            self.0.close()
        }
    }

    fn open<E: Engine>() -> Emulated<E> {
        Emulated::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &Options::small_for_tests()).unwrap()
    }

    fn commit<E: Engine>(db: &Emulated<E>, model: &mut Model, b: &WriteBatch) -> u64 {
        let seq = db.commit(b).unwrap();
        apply(model, b);
        assert_eq!(read_all(&db.snapshot().unwrap()).unwrap(), *model);
        seq
    }

    #[test]
    fn ranges_become_point_deletes_in_batch_order() {
        let db = open::<PointsOnly>();
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        for k in ["a", "b", "c", "d", "e"] {
            b.put(k, "0");
        }
        assert_eq!(commit(&db, &mut model, &b), 1);
        let mut b = WriteBatch::new();
        b.put("bb", "1").put("f", "1").delete_range("b", "d").put("c", "2").delete_range("e", "z");
        assert_eq!(commit(&db, &mut model, &b), 2);
        let keys: Vec<_> = model.keys().map(|k| String::from_utf8(k.clone()).unwrap()).collect();
        assert_eq!(keys, ["a", "c", "d"]);
        let mut expected = WriteBatch::new();
        expected.put("bb", "1").put("f", "1").delete("b").delete("bb").delete("c").put("c", "2");
        expected.delete("e").delete("f");
        assert_eq!(db.inner().log.lock().last(), Some(&expected));
    }

    #[test]
    fn every_non_empty_batch_gets_a_sequence() {
        let db = open::<PointsOnly>();
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        b.delete_range("x", "x");
        assert_eq!(commit(&db, &mut model, &b), 1, "only an empty range, on an empty database");
        let mut b = WriteBatch::new();
        b.put("", "empty key").put("\0", "nul");
        assert_eq!(commit(&db, &mut model, &b), 2);
        let mut b = WriteBatch::new();
        b.delete_range("q", "q").delete_range("", "");
        assert_eq!(commit(&db, &mut model, &b), 3, "only empty ranges, the first probed keys exist");
        let mut b = WriteBatch::new();
        b.delete_range("m", "n");
        assert_eq!(commit(&db, &mut model, &b), 4, "a range that covers nothing");
        assert_eq!(db.commit(&WriteBatch::new()).unwrap(), 4, "an empty batch is not a commit");
        assert_eq!(model.len(), 2);
    }

    #[test]
    fn the_rewrite_never_introduces_the_empty_key() {
        let db = open::<NoEmptyKey>();
        let mut model = Model::new();
        let mut seq = 0;
        let mut check = |b: &WriteBatch, rewritten: Option<&WriteBatch>, what: &str| {
            seq += 1;
            assert_eq!(commit(&db, &mut model, b), seq, "{what}");
            if let Some(want) = rewritten {
                assert_eq!(db.inner().0.log.lock().last(), Some(want), "{what}");
            }
        };
        let mut nul = WriteBatch::new();
        nul.delete("\0");
        let mut b = WriteBatch::new();
        b.delete_range("x", "x");
        check(&b, Some(&nul), "only an empty range, on an empty database");
        let mut b = WriteBatch::new();
        b.delete_range("", "");
        check(&b, Some(&nul), "the empty range at the empty key");
        let mut b = WriteBatch::new();
        b.delete_range("", "a");
        check(&b, Some(&nul), "a range from the empty key that covers nothing");
        let mut b = WriteBatch::new();
        b.put("\0", "nul").put("a", "1").put("b", "2");
        check(&b, None, "keys");
        let mut b = WriteBatch::new();
        b.delete_range("q", "q").delete_range("", "");
        let mut want = WriteBatch::new();
        want.delete("\0\0");
        check(&b, Some(&want), "only empty ranges, \"\\0\" exists");
        let mut b = WriteBatch::new();
        b.delete_range("", "b");
        let mut want = WriteBatch::new();
        want.delete("\0").delete("a");
        check(&b, Some(&want), "a range from the empty key over existing keys");
        let mut b = WriteBatch::new();
        b.delete_range("", "\0");
        check(&b, Some(&nul), "a range that covers only the empty key, which does not exist");
        let mut b = WriteBatch::new();
        b.delete_range("", "z");
        check(&b, None, "everything");
        assert!(model.is_empty());
    }

    /// The empty key is deleted like any other key when it is stored (or
    /// put earlier in the batch): the engine accepted it, so it accepts its
    /// deletion.
    #[test]
    fn an_existing_empty_key_is_deleted_like_any_other() {
        let db = open::<PointsOnly>();
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        b.put("", "v").put("b", "1");
        commit(&db, &mut model, &b);
        let mut b = WriteBatch::new();
        b.delete_range("", "a");
        commit(&db, &mut model, &b);
        let mut want = WriteBatch::new();
        want.delete("");
        assert_eq!(db.inner().log.lock().last(), Some(&want), "the stored empty key");
        let mut b = WriteBatch::new();
        b.put("", "w").delete_range("", "\0");
        commit(&db, &mut model, &b);
        let mut want = WriteBatch::new();
        want.put("", "w").delete("");
        assert_eq!(db.inner().log.lock().last(), Some(&want), "the empty key put earlier in the batch");
        assert_eq!(model.keys().collect::<Vec<_>>(), [b"b"]);
    }

    #[test]
    fn invalid_ranges_are_rejected_before_anything_happens() {
        let db = open::<PointsOnly>();
        let mut b = WriteBatch::new();
        b.put("a", "1").delete_range("z", "a");
        assert!(matches!(db.commit(&b), Err(Error::InvalidArgument(_))));
        let mut b = WriteBatch::new();
        b.delete_range(vec![b'k'; MAX_KEY_LEN + 1], "z");
        assert!(matches!(db.commit(&b), Err(Error::InvalidArgument(_))));
        assert!(db.inner().log.lock().is_empty());
        assert_eq!(db.snapshot().unwrap().seq(), 0);
    }

    #[test]
    fn wide_ranges_scan_every_page() {
        let db = open::<PointsOnly>();
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        for i in 0..3 * SCAN_PAGE + 7 {
            b.put(format!("k{i:06}"), "v");
        }
        commit(&db, &mut model, &b);
        let mut b = WriteBatch::new();
        b.delete_range("k000003", "k003000");
        commit(&db, &mut model, &b);
        assert_eq!(model.len(), 3 + (3 * SCAN_PAGE + 7 - 3000));
        assert_eq!(db.inner().log.lock().last().unwrap().len(), 3000 - 3, "one deletion per key in the range");
    }

    /// A transaction commits the same batch the writes made, with its ranges
    /// rewritten at their position, in one commit of the wrapped engine.
    #[test]
    fn txn_commit_rewrites_ranges_like_a_batch() {
        let db = open::<PointsOnly>();
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        for k in ["a", "b", "c", "d", "e"] {
            b.put(k, "0");
        }
        commit(&db, &mut model, &b);
        let mut t = db.begin_write().unwrap();
        t.put(b"bb", b"1").unwrap();
        t.delete_range(b"b", b"d").unwrap();
        t.put(b"c", b"2").unwrap();
        assert_eq!(t.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap().len(), 4, "a, c, d, e");
        assert_eq!(db.inner().log.lock().len(), 1, "nothing reaches the engine before the commit");
        assert_eq!(t.commit().unwrap(), 2);
        let mut expected = WriteBatch::new();
        expected.put("bb", "1").delete("b").delete("bb").delete("c").put("c", "2");
        assert_eq!(db.inner().log.lock().last(), Some(&expected));
        assert_eq!(db.inner().log.lock().len(), 2, "one commit for the transaction");
    }

    #[test]
    fn txn_without_writes_does_not_touch_the_engine() {
        let db = open::<PointsOnly>();
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        b.put("a", "1");
        commit(&db, &mut model, &b);
        let mut t = db.begin_write().unwrap();
        assert!(matches!(t.put(&vec![b'k'; MAX_KEY_LEN + 1], b"v"), Err(Error::InvalidArgument(_))));
        assert!(matches!(t.delete_range(b"z", b"a"), Err(Error::InvalidArgument(_))));
        assert_eq!(t.commit().unwrap(), 1);
        let mut t = db.begin_write().unwrap();
        t.put(b"x", b"1").unwrap();
        t.abort();
        assert_eq!(db.inner().log.lock().len(), 1);
        assert_eq!(db.snapshot().unwrap().seq(), 1);
    }

    #[test]
    fn txn_rewrite_never_introduces_the_empty_key() {
        let db = open::<NoEmptyKey>();
        let mut t = db.begin_write().unwrap();
        t.delete_range(b"", b"a").unwrap();
        assert_eq!(t.commit().unwrap(), 1, "a range that covers nothing is still a write");
        let mut want = WriteBatch::new();
        want.delete("\0");
        assert_eq!(db.inner().0.log.lock().last(), Some(&want));
    }

    /// A scan whose first pages of the snapshot are hidden by the
    /// transaction's ranges still fills its limit, and the written keys merge
    /// into the right place.
    #[test]
    fn txn_scan_reads_past_hidden_pages() {
        let db = open::<PointsOnly>();
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        for i in 0..4 * SCAN_PAGE {
            b.put(format!("k{i:06}"), "v");
        }
        commit(&db, &mut model, &b);
        let mut t = db.begin_write().unwrap();
        t.delete_range(b"k000002", b"k003900").unwrap();
        t.put(b"k000500", b"mine").unwrap();
        t.put(b"k003950x", b"mine").unwrap();
        let rows = t.scan(Bound::Unbounded, Bound::Unbounded, 6).unwrap();
        let keys: Vec<_> = rows.iter().map(|(k, _)| String::from_utf8(k.clone()).unwrap()).collect();
        assert_eq!(keys, ["k000000", "k000001", "k000500", "k003900", "k003901", "k003902"]);
        let rows = t.scan(Bound::Excluded(b"k003950"), Bound::Unbounded, 3).unwrap();
        let keys: Vec<_> = rows.iter().map(|(k, _)| String::from_utf8(k.clone()).unwrap()).collect();
        assert_eq!(keys, ["k003950x", "k003951", "k003952"]);
        assert!(t.scan(Bound::Included(b"k001000"), Bound::Excluded(b"k000999"), 5).unwrap().is_empty(), "inverted");
        assert!(t.scan(Bound::Excluded(b"k000500"), Bound::Excluded(b"k000500"), 5).unwrap().is_empty());
    }

    #[test]
    fn name_is_derived_from_the_inner_engine() {
        assert_eq!(Emulated::<LogEngine>::name(), "log-emulated");
        assert!(std::ptr::eq(Emulated::<LogEngine>::name(), Emulated::<LogEngine>::name()));
        assert_eq!(Emulated::<PointsOnly>::name(), "points-emulated");
    }
}
