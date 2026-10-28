//! Leveled compaction: policy (what to compact next), the merge itself and
//! the resulting level edit.
//!
//! * L0 is compacted when it holds `l0_trigger` files: all L0 files plus the
//!   overlapping L1 files are merged into L1.
//! * Ln (n >= 1) is compacted when its size exceeds its target
//!   (`level_base_bytes * level_multiplier^(n-1)`): one file, chosen
//!   round-robin, plus the overlapping Ln+1 files are merged into Ln+1. A
//!   file without overlap is moved down without rewriting it.
//!
//! The thresholds come from [`CompactionConfig`](crate::config::CompactionConfig);
//! the constants below are its M0 values.
//! * The merge keeps only the newest version of each key. Snapshots that
//!   need older versions keep reading the input files through their own
//!   `Version`. A tombstone is dropped when no level below the output level
//!   has a file whose key range contains the key.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kerno_core::{Error, Result};

use crate::config::{Config, MIN_WRITE_BUFFER, SlowdownPolicy};
use crate::iter::{InternalIterator, LevelIter, MergeIter};
use crate::sst::{Table, TableBuilder, TableCtx};
use crate::stats::{Stats, bump};
use crate::version::Levels;

pub const NUM_LEVELS: usize = 7;
pub const L0_COMPACTION_TRIGGER: usize = 4;
/// Each commit is delayed (up to 1 ms) while L0 has this many files.
pub const L0_SLOWDOWN: usize = 8;
/// Writers wait while L0 has this many files.
pub const L0_STOP: usize = 12;
/// Writers wait while this many memtables are waiting to be flushed.
pub const MAX_IMMUTABLE: usize = 4;
pub const LEVEL_SIZE_RATIO: u64 = 10;
pub const BLOCK_SIZE: usize = 4096;

/// Sizes and thresholds derived from `Config`.
#[derive(Clone, Debug)]
pub struct Tuning {
    pub write_buffer: usize,
    pub target_file_size: u64,
    pub l1_max_bytes: u64,
    pub l0_trigger: usize,
    pub l0_slowdown: usize,
    pub l0_stop: usize,
    pub level_multiplier: u64,
    pub slowdown: SlowdownPolicy,
}

impl Tuning {
    pub fn new(cfg: &Config) -> Self {
        let c = &cfg.compaction;
        let write_buffer = cfg.write_buffer_bytes.max(MIN_WRITE_BUFFER);
        let wb = write_buffer as u64;
        Tuning {
            write_buffer,
            target_file_size: c.target_file_size.unwrap_or(wb),
            l1_max_bytes: c.level_base_bytes.unwrap_or_else(|| (L0_COMPACTION_TRIGGER as u64).saturating_mul(wb)),
            l0_trigger: c.l0_trigger,
            l0_slowdown: c.l0_slowdown,
            l0_stop: c.l0_stop,
            level_multiplier: c.level_multiplier,
            slowdown: c.slowdown,
        }
    }

    pub fn level_max_bytes(&self, level: usize) -> u64 {
        debug_assert!(level >= 1);
        self.l1_max_bytes.saturating_mul(self.level_multiplier.saturating_pow(level as u32 - 1))
    }
}

pub struct Plan {
    /// Input level; the output goes to `level + 1`.
    pub level: usize,
    /// Files from `level` (L0: newest first).
    pub inputs: Vec<Arc<Table>>,
    /// Overlapping files from `level + 1` (sorted).
    pub next_inputs: Vec<Arc<Table>>,
    /// Levels below the output level, for the tombstone check.
    pub deeper: Levels,
}

impl Plan {
    pub fn is_trivial_move(&self) -> bool {
        self.level > 0 && self.inputs.len() == 1 && self.next_inputs.is_empty()
    }

    pub fn largest_input_key(&self) -> &[u8] {
        self.inputs.iter().map(|t| t.largest.as_slice()).max().unwrap_or_default()
    }

    pub fn all_inputs(&self) -> impl Iterator<Item = &Arc<Table>> {
        self.inputs.iter().chain(&self.next_inputs)
    }
}

fn level_bytes(level: &[Arc<Table>]) -> u64 {
    level.iter().map(|t| t.size).sum()
}

/// Picks the most urgent compaction, if any level is over its target.
/// `pointers[n]` is the largest key of the previous compaction out of Ln.
pub fn pick(levels: &Levels, tuning: &Tuning, pointers: &[Vec<u8>]) -> Option<Plan> {
    let mut best: Option<(f64, usize)> = None;
    let l0 = levels[0].len() as f64 / tuning.l0_trigger as f64;
    if l0 >= 1.0 {
        best = Some((l0, 0));
    }
    for (n, files) in levels.iter().enumerate().take(NUM_LEVELS - 1).skip(1) {
        let score = level_bytes(files) as f64 / tuning.level_max_bytes(n) as f64;
        if score >= 1.0 && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, n));
        }
    }
    let (_, level) = best?;
    let inputs = if level == 0 {
        levels[0].clone()
    } else {
        let files = &levels[level];
        let after = files.iter().position(|t| t.largest > pointers[level]).unwrap_or(0);
        vec![files[after].clone()]
    };
    let lo = inputs.iter().map(|t| t.smallest.as_slice()).min()?;
    let hi = inputs.iter().map(|t| t.largest.as_slice()).max()?;
    let next_inputs = levels[level + 1].iter().filter(|t| t.overlaps(lo, hi)).cloned().collect();
    Some(Plan { level, inputs, next_inputs, deeper: levels[level + 2..].to_vec() })
}

/// Bytes that leveled compaction still has to move: for every level from L1
/// to the second-to-last (the last one is never compacted), the bytes above
/// its target, plus all of L0 once it has `l0_trigger` files.
pub fn pending_bytes(levels: &Levels, tuning: &Tuning) -> u64 {
    let mut pending = 0u64;
    if levels[0].len() >= tuning.l0_trigger {
        pending = level_bytes(&levels[0]);
    }
    for (n, files) in levels.iter().enumerate().take(NUM_LEVELS - 1).skip(1) {
        pending = pending.saturating_add(level_bytes(files).saturating_sub(tuning.level_max_bytes(n)));
    }
    pending
}

/// The largest number of L0 files whose key ranges contain one same key.
pub fn l0_depth(files: &[Arc<Table>]) -> usize {
    // Closed ranges: at equal keys, starts are counted before ends.
    let mut events: Vec<(&[u8], bool)> = Vec::with_capacity(files.len() * 2);
    for t in files {
        events.push((t.smallest.as_slice(), false));
        events.push((t.largest.as_slice(), true));
    }
    events.sort_unstable();
    let (mut open, mut depth) = (0usize, 0usize);
    for (_, is_end) in events {
        if is_end {
            open -= 1;
        } else {
            open += 1;
            depth = depth.max(open);
        }
    }
    depth
}

/// Answers "could a level below the output contain this key?" for keys
/// presented in increasing order.
struct DeeperLevels<'a> {
    levels: &'a [Vec<Arc<Table>>],
    cursors: Vec<usize>,
}

impl<'a> DeeperLevels<'a> {
    fn new(levels: &'a [Vec<Arc<Table>>]) -> Self {
        DeeperLevels { levels, cursors: vec![0; levels.len()] }
    }

    fn may_contain(&mut self, key: &[u8]) -> bool {
        for (files, cur) in self.levels.iter().zip(&mut self.cursors) {
            while *cur < files.len() && files[*cur].largest.as_slice() < key {
                *cur += 1;
            }
            if *cur < files.len() && files[*cur].smallest.as_slice() <= key {
                return true;
            }
        }
        false
    }
}

/// Merges the plan's inputs into new, synced tables (not yet referenced by
/// the MANIFEST). Returns `None` if `abort` was raised; partial outputs are
/// removed in that case and on error.
pub fn run(
    ctx: &Arc<TableCtx>,
    plan: &Plan,
    tuning: &Tuning,
    next_file: &AtomicU64,
    abort: &AtomicBool,
    stats: &Stats,
) -> Result<Option<Vec<Arc<Table>>>> {
    let mut outputs: Vec<Table> = Vec::new();
    let mut builder: Option<TableBuilder> = None;
    let result = merge_into(ctx, plan, tuning, next_file, abort, stats, &mut outputs, &mut builder);
    match result {
        Ok(true) => Ok(Some(outputs.into_iter().map(Arc::new).collect())),
        Ok(false) | Err(_) => {
            if let Some(b) = builder {
                b.abandon();
            }
            for t in &outputs {
                t.mark_obsolete(); // deleted on drop
            }
            drop(outputs);
            result.map(|_| None)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn merge_into(
    ctx: &Arc<TableCtx>,
    plan: &Plan,
    tuning: &Tuning,
    next_file: &AtomicU64,
    abort: &AtomicBool,
    stats: &Stats,
    outputs: &mut Vec<Table>,
    builder: &mut Option<TableBuilder>,
) -> Result<bool> {
    let mut children: Vec<Box<dyn InternalIterator + '_>> = Vec::new();
    if plan.level == 0 {
        for t in &plan.inputs {
            children.push(Box::new(t.iter(false)));
        }
    } else {
        children.push(Box::new(LevelIter::new(&plan.inputs, false)));
    }
    children.push(Box::new(LevelIter::new(&plan.next_inputs, false)));
    let mut it = MergeIter::new(children);
    let mut deeper = DeeperLevels::new(&plan.deeper);
    it.seek_to_first()?;
    let mut n = 0u64;
    while it.valid() {
        n += 1;
        if n.is_multiple_of(512) && abort.load(Ordering::Relaxed) {
            return Ok(false);
        }
        if it.value().is_none() && !deeper.may_contain(it.key()) {
            bump(&stats.tombstones_dropped, 1);
            it.next()?;
            continue;
        }
        if builder.is_none() {
            *builder = Some(TableBuilder::create(ctx, next_file.fetch_add(1, Ordering::SeqCst))?);
        }
        let b = builder.as_mut().expect("just created");
        b.add(it.key(), it.seq(), it.value())?;
        if b.estimated_size() >= tuning.target_file_size {
            outputs.push(builder.take().expect("just used").finish()?);
        }
        it.next()?;
    }
    if let Some(b) = builder.take() {
        outputs.push(b.finish()?);
    }
    Ok(true)
}

/// The levels after replacing the plan's inputs with `outputs` in
/// `level + 1`. `levels` may have gained newer L0 files since the plan was
/// picked; they stay where they are.
pub fn apply(levels: &Levels, plan: &Plan, outputs: &[Arc<Table>]) -> Result<Levels> {
    let removed: HashSet<u64> = plan.all_inputs().map(|t| t.number).collect();
    let mut new = levels.clone();
    let before = new[plan.level].len() + new[plan.level + 1].len();
    new[plan.level].retain(|t| !removed.contains(&t.number));
    new[plan.level + 1].retain(|t| !removed.contains(&t.number));
    if before - new[plan.level].len() - new[plan.level + 1].len() != removed.len() {
        return Err(Error::Corruption("internal: compaction inputs are no longer in the version".into()));
    }
    let out = &mut new[plan.level + 1];
    out.extend(outputs.iter().cloned());
    out.sort_by(|a, b| a.smallest.cmp(&b.smallest));
    if out.windows(2).any(|w| w[0].largest >= w[1].smallest) {
        return Err(Error::Corruption(format!(
            "internal: compaction produced overlapping files in L{}",
            plan.level + 1
        )));
    }
    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_core::contract::Options;

    #[test]
    fn level_targets_grow_by_the_ratio() {
        let t = Tuning::new(&Config::from(&Options::small_for_tests()));
        assert_eq!(t.level_max_bytes(1), 4 * (16 << 10));
        assert_eq!(t.level_max_bytes(2), 40 * (16 << 10));
        assert_eq!(t.level_max_bytes(6), 400_000 * (16 << 10));
        assert_eq!((t.l0_trigger, t.l0_slowdown, t.l0_stop), (4, 8, 12));
        assert_eq!(t.target_file_size, 16 << 10);
    }

    #[test]
    fn level_targets_follow_the_compaction_config() {
        let mut cfg = Config::from(&Options::small_for_tests());
        cfg.compaction.level_base_bytes = Some(1000);
        cfg.compaction.level_multiplier = 3;
        let t = Tuning::new(&cfg);
        assert_eq!(t.level_max_bytes(1), 1000);
        assert_eq!(t.level_max_bytes(3), 9000);
    }

    #[test]
    fn struct_update_configs_keep_the_m0_geometry() {
        // A Config built by struct update from Config::default() derives
        // its sizes from its own write buffer, as M0 did.
        let cfg = Config { write_buffer_bytes: 16 << 10, ..Config::default() };
        let t = Tuning::new(&cfg);
        assert_eq!(t.target_file_size, 16 << 10);
        assert_eq!(t.level_max_bytes(1), 64 << 10);
        let from_opts = Tuning::new(&Config::from(&Options::small_for_tests()));
        assert_eq!((t.target_file_size, t.l1_max_bytes), (from_opts.target_file_size, from_opts.l1_max_bytes));
        // Explicit sizes still win.
        let mut cfg = cfg;
        cfg.compaction.target_file_size = Some(1 << 20);
        assert_eq!(Tuning::new(&cfg).target_file_size, 1 << 20);
    }

    fn table(ctx: &Arc<TableCtx>, number: u64, lo: &str, hi: &str) -> Arc<Table> {
        let mut b = TableBuilder::create(ctx, number).unwrap();
        b.add(lo.as_bytes(), 1, Some(&[0u8; 100])).unwrap();
        if hi != lo {
            b.add(hi.as_bytes(), 1, Some(&[0u8; 100])).unwrap();
        }
        Arc::new(b.finish().unwrap())
    }

    fn ctx() -> Arc<TableCtx> {
        let vfs = kerno_testkit::vfs::SimVfs::new(1).as_vfs();
        vfs.create_dir_all(std::path::Path::new("/t")).unwrap();
        let stats = Arc::new(crate::stats::Stats::default());
        Arc::new(TableCtx::new(vfs, "/t".into(), 1 << 20, stats, BLOCK_SIZE))
    }

    #[test]
    fn l0_depth_counts_overlapping_ranges() {
        let ctx = ctx();
        assert_eq!(l0_depth(&[]), 0);
        let a = table(&ctx, 1, "a", "c");
        let b = table(&ctx, 2, "c", "e"); // touches `a` at "c"
        let c = table(&ctx, 3, "f", "g");
        let d = table(&ctx, 4, "a", "z");
        assert_eq!(l0_depth(std::slice::from_ref(&a)), 1);
        assert_eq!(l0_depth(&[a.clone(), c.clone()]), 1);
        assert_eq!(l0_depth(&[a.clone(), b.clone()]), 2);
        assert_eq!(l0_depth(&[a, b, c, d]), 3);
    }

    #[test]
    fn pending_bytes_counts_l0_and_excess_of_deeper_levels() {
        let ctx = ctx();
        let mut cfg = Config::from(&Options::small_for_tests());
        cfg.compaction.l0_trigger = 2;
        cfg.compaction.l0_stop = 2;
        let t = Tuning { l1_max_bytes: 1, ..Tuning::new(&cfg) };
        let mut levels: Levels = vec![Vec::new(); NUM_LEVELS];
        assert_eq!(pending_bytes(&levels, &t), 0);
        levels[0].push(table(&ctx, 1, "a", "b"));
        // One L0 file is below the trigger.
        assert_eq!(pending_bytes(&levels, &t), 0);
        levels[0].push(table(&ctx, 2, "a", "b"));
        let l0 = level_bytes(&levels[0]);
        assert_eq!(pending_bytes(&levels, &t), l0);
        // L1 holds 1 byte at most; L2 10; the last level is never pending.
        levels[1].push(table(&ctx, 3, "c", "d"));
        levels[2].push(table(&ctx, 4, "e", "f"));
        levels[NUM_LEVELS - 1].push(table(&ctx, 5, "g", "h"));
        let expected = l0 + (levels[1][0].size - 1) + (levels[2][0].size - 10);
        assert_eq!(pending_bytes(&levels, &t), expected);
    }
}
