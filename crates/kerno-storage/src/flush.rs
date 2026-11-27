//! Writing memtables to L0.
//!
//! A memtable of a workload that writes several key prefixes (`mem/<id>` and
//! `emb/<id>` with ids growing in time) spans from its lowest to its highest
//! key, so as one L0 file it overlaps every L1 file in between, and the L0
//! compaction that follows rewrites all of them (phase1-design.md §5.4, H1).
//! With `flush_split` the output is cut wherever an L1 file lies entirely
//! between two consecutive keys: the pieces of one flush are key-disjoint, each
//! overlaps only the L1 files it really intersects, and the L0 depth metric
//! counts them once.
//!
//! The range tombstones of the memtable go to the tables as fragments. A
//! point a fragment of the same memtable covers with a higher sequence is not
//! written (SST-F2, SST-V4); the fragments are cut where the output is, so
//! each table holds the pieces that lie within the keys it stands for.

use std::borrow::Cow;
use std::sync::Arc;

use kerno_core::{Error, Result};

use crate::iter::{InternalIterator, MergeIter};
use crate::memtable::Memtable;
use crate::rangedel::{CoverCursor, FragmentSplitter};
use crate::sst::{Table, TableBuilder, TableCtx};

/// Finds the cuts of a flush: keys are presented in increasing order.
struct SplitPoints<'a> {
    l1: &'a [Arc<Table>],
    /// The first L1 file that starts after the previous key.
    next: usize,
}

impl<'a> SplitPoints<'a> {
    fn new(l1: &'a [Arc<Table>]) -> Self {
        SplitPoints { l1, next: 0 }
    }

    /// Whether some L1 file lies entirely inside `(prev, key)`. L1 is sorted
    /// and key-disjoint, so if any file does, the first one that starts after
    /// `prev` does.
    fn cut_between(&mut self, prev: &[u8], key: &[u8]) -> bool {
        while self.next < self.l1.len() && self.l1[self.next].smallest.as_slice() <= prev {
            self.next += 1;
        }
        self.l1.get(self.next).is_some_and(|t| t.ends_before(key))
    }
}

/// Writes the newest version of every key in `mem` to new synced tables,
/// newest keys last. With a non-empty `l1` (the sorted L1 files) the output is
/// cut around the L1 files that lie entirely between two consecutive keys;
/// otherwise it is one table. `next_number` supplies the file numbers. The
/// tables are not yet referenced by the MANIFEST and their directory entries
/// are not yet durable: the caller syncs the directory. A memtable without
/// entries gives no table. On failure the partial output is removed.
/// `reason` is recorded as each table's `creation_reason` (v1.md §7.6).
/// `closing` is the close-time flush: nothing else is writing, so its tables
/// skip the intermediate syncs and, for a memtable above
/// `CLOSE_LZ4_MAX_BYTES`, the LZ4 pass (they keep `BLOCK_LZ4` and
/// store every block with codec 0; the pass was a quarter of a close, and the
/// syncs slowed it more, see the report of 2026-11-24).
pub fn write_memtable(
    ctx: &Arc<TableCtx>,
    mem: &Memtable,
    reason: u8,
    l1: &[Arc<Table>],
    closing: bool,
    next_number: &mut dyn FnMut() -> u64,
) -> Result<Vec<Table>> {
    let mut tables: Vec<Table> = Vec::new();
    let mut builder: Option<TableBuilder> = None;
    let result = fill(ctx, mem, reason, l1, closing, next_number, &mut tables, &mut builder);
    if result.is_err() {
        if let Some(b) = builder {
            b.abandon();
        }
        for t in &tables {
            t.mark_obsolete(); // deleted on drop
        }
        tables.clear();
    }
    result.map(|()| tables)
}

/// The largest memtable a close-time flush still compresses. Below it the LZ4
/// pass costs a few milliseconds and the table, which for a small database is
/// the whole database, comes out about half the size; above it the pass is a
/// visible part of the close and compaction compresses the data later anyway.
const CLOSE_LZ4_MAX_BYTES: usize = 8 << 20;

fn create(ctx: &Arc<TableCtx>, number: u64, reason: u8, closing: bool, lz4: bool) -> Result<TableBuilder> {
    let b = TableBuilder::create(ctx, number, reason)?;
    let b = if closing { b.without_intermediate_syncs() } else { b };
    Ok(if lz4 { b } else { b.without_lz4_pass() })
}

#[allow(clippy::too_many_arguments)]
fn fill(
    ctx: &Arc<TableCtx>,
    mem: &Memtable,
    reason: u8,
    l1: &[Arc<Table>],
    closing: bool,
    next_number: &mut dyn FnMut() -> u64,
    tables: &mut Vec<Table>,
    builder: &mut Option<TableBuilder>,
) -> Result<()> {
    let lz4 = !closing || mem.approx_bytes() <= CLOSE_LZ4_MAX_BYTES;
    if let Some((seq, points)) = mem.frozen_points() {
        // One commit, sorted, each key once, no tombstone: the points are the
        // output as they are (same tables as the merge path below gives).
        let mut split = SplitPoints::new(l1);
        let mut prev: &[u8] = &[];
        for (key, value) in points {
            let key = key.as_slice();
            if builder.is_some() && split.cut_between(prev, key) {
                tables.push(builder.take().expect("checked above").finish_with(&[])?);
            }
            if builder.is_none() {
                *builder = Some(create(ctx, next_number(), reason, closing, lz4)?);
            }
            builder.as_mut().expect("just created").add(key, seq, value.as_deref())?;
            prev = key;
        }
        if let Some(b) = builder.take() {
            tables.push(b.finish_with(&[])?);
        }
        return Ok(());
    }
    let fragments = mem.fragments(u64::MAX).unwrap_or_default();
    let mut cover = CoverCursor::new(vec![Cow::Borrowed(&fragments[..])]);
    cover.seek(&[]);
    let mut splitter = FragmentSplitter::new(fragments.to_vec());
    let mut it = MergeIter::new(vec![Box::new(mem.iter(u64::MAX))]);
    let mut split = SplitPoints::new(l1);
    let mut prev: Vec<u8> = Vec::new();
    it.seek_to_first()?;
    while it.valid() {
        // Covered by a newer tombstone of this memtable: the fragment, which
        // the table keeps, hides it (and whatever is older, below).
        if cover.covering(it.key()) > it.seq() {
            it.next()?;
            continue;
        }
        if builder.is_some() && split.cut_between(&prev, it.key()) {
            let pieces = splitter.take_below(Some(it.key()));
            tables.push(builder.take().expect("checked above").finish_with(&pieces)?);
        }
        if builder.is_none() {
            *builder = Some(create(ctx, next_number(), reason, closing, lz4)?);
        }
        builder.as_mut().expect("just created").add(it.key(), it.seq(), it.value())?;
        prev.clear();
        prev.extend_from_slice(it.key());
        it.next()?;
    }
    let rest = splitter.take_below(None);
    if builder.is_none() && !rest.is_empty() {
        // Only tombstones left.
        *builder = Some(create(ctx, next_number(), reason, closing, lz4)?);
    }
    if let Some(b) = builder.take() {
        tables.push(b.finish_with(&rest)?);
    }
    Ok::<(), Error>(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::compaction::BLOCK_SIZE;
    use crate::format::props::REASON_FLUSH;
    use crate::format::wal::{CommitOp, canonicalize};
    use crate::stats::Stats;

    fn ctx() -> Arc<TableCtx> {
        let vfs = kerno_testkit::vfs::SimVfs::new(1).as_vfs();
        vfs.create_dir_all(std::path::Path::new("/t")).unwrap();
        Arc::new(TableCtx::new(vfs, "/t".into(), 1 << 20, Arc::new(Stats::default()), BLOCK_SIZE))
    }

    fn l1_table(ctx: &Arc<TableCtx>, number: u64, lo: &str, hi: &str) -> Arc<Table> {
        let mut b = TableBuilder::create(ctx, number, REASON_FLUSH).unwrap();
        b.add(lo.as_bytes(), 1, Some(b"x")).unwrap();
        b.add(hi.as_bytes(), 1, Some(b"x")).unwrap();
        Arc::new(b.finish().unwrap())
    }

    fn memtable(keys: &[&str]) -> Memtable {
        let mem = Memtable::new(0);
        for (i, k) in keys.iter().enumerate() {
            mem.apply_commit(i as u64 + 1, &canonicalize([CommitOp::Put { key: k.as_bytes(), value: b"v" }]));
        }
        mem
    }

    fn ranges(tables: &[Table]) -> Vec<(String, String)> {
        let s = |b: &[u8]| String::from_utf8(b.to_vec()).unwrap();
        tables.iter().map(|t| (s(&t.smallest), s(&t.largest))).collect()
    }

    fn write(ctx: &Arc<TableCtx>, mem: &Memtable, l1: &[Arc<Table>]) -> Vec<Table> {
        static NEXT: AtomicU64 = AtomicU64::new(100);
        write_memtable(ctx, mem, REASON_FLUSH, l1, false, &mut || NEXT.fetch_add(1, Ordering::SeqCst)).unwrap()
    }

    #[test]
    fn split_points_need_an_l1_file_entirely_between_two_keys() {
        let ctx = ctx();
        let l1 = [l1_table(&ctx, 1, "c", "d"), l1_table(&ctx, 2, "m", "p")];
        let mem = memtable(&["a", "b", "e", "f", "m", "n", "z"]);
        // c..d lies between b and e: a cut there. m..p holds the keys m and
        // n of the memtable, so it lies entirely between no two of them.
        assert_eq!(ranges(&write(&ctx, &mem, &l1)), [("a".into(), "b".into()), ("e".into(), "z".into())]);
        // Without L1 files, or with the split off (no files given), one table.
        assert_eq!(ranges(&write(&ctx, &mem, &[])), [("a".into(), "z".into())]);
    }

    #[test]
    fn every_file_between_keys_cuts_once() {
        let ctx = ctx();
        let l1 = [l1_table(&ctx, 1, "c", "d"), l1_table(&ctx, 2, "g", "h"), l1_table(&ctx, 3, "w", "x")];
        let mem = memtable(&["a", "k", "l"]);
        // c..d and g..h both lie between a and k: one cut. w..x is past the
        // last key: no empty table.
        assert_eq!(ranges(&write(&ctx, &mem, &l1)), [("a".into(), "a".into()), ("k".into(), "l".into())]);
    }

    type Entry = (Vec<u8>, u64, Option<Vec<u8>>);

    fn contents(tables: &[Table]) -> Vec<Vec<Entry>> {
        tables
            .iter()
            .map(|t| {
                let mut it = t.iter(false);
                it.seek_to_first().unwrap();
                let mut out = Vec::new();
                while it.valid() {
                    out.push((it.key().to_vec(), it.seq(), it.value().map(<[u8]>::to_vec)));
                    it.next().unwrap();
                }
                out
            })
            .collect()
    }

    #[test]
    fn frozen_fast_path_writes_what_the_merge_path_writes() {
        let ctx = ctx();
        let l1 = [l1_table(&ctx, 1, "c", "d"), l1_table(&ctx, 2, "m", "p")];
        let commit = Arc::new(
            canonicalize([
                CommitOp::Put { key: b"z", value: b"9" },
                CommitOp::Put { key: b"a", value: b"1" },
                CommitOp::Delete { key: b"b" },
                CommitOp::Put { key: b"e", value: b"" },
                CommitOp::Put { key: b"n", value: b"5" },
                CommitOp::Put { key: b"a", value: b"2" },
            ])
            .to_owned_commit(),
        );
        let normal = Memtable::new(0);
        normal.apply_commit(7, &commit);
        let frozen = Memtable::frozen(0, 7, commit.clone());
        assert!(frozen.frozen_points().is_some() && normal.frozen_points().is_none());
        for l1 in [&l1[..], &[][..]] {
            let (a, b) = (write(&ctx, &normal, l1), write(&ctx, &frozen, l1));
            assert_eq!(ranges(&a), ranges(&b));
            assert_eq!(contents(&a), contents(&b));
            assert_eq!(
                a.iter().map(Table::entries).collect::<Vec<_>>(),
                b.iter().map(Table::entries).collect::<Vec<_>>()
            );
        }
    }
}
