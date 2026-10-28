//! Leveled compaction: policy (what to compact next), the merge itself and
//! the resulting level edit.
//!
//! * L0 is compacted when its metric (files, or depth with the condition-1
//!   levers) reaches `l0_trigger`: all L0 files, or with `l0_partial` the
//!   oldest suffix that fits `max_compaction_bytes`, plus the overlapping L1
//!   files are merged into L1. The oldest L0 file moves down without a
//!   rewrite when it overlaps no L1 file and `l0_trivial_move` is on.
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
//! * Range tombstones travel as fragments. The merge drops a point that a
//!   fragment of its inputs covers with a higher sequence, cuts the fragments
//!   it keeps at every output boundary (size, grandparents, subcompaction
//!   ranges), and drops a fragment when no file below the output level
//!   overlaps it (phase1-design.md §5.5). A table with fragments is never
//!   moved down without being rewritten: the rewrite is where they go away.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Bound;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use kerno_core::{Error, Result};

use crate::config::{Background, Config, L0Metric, MIN_WRITE_BUFFER, SlowdownPolicy};
use crate::format::props::{REASON_COMPACTION, REASON_DIRECTED};
use crate::format::rangedel::Fragment;
use crate::iter::{InternalIterator, LevelIter, MergeIter};
use crate::rangedel::{self, CoverCursor, FragmentSplitter};
use crate::sst::{Table, TableBuilder, TableCtx};
use crate::stats::{Stats, bump};
use crate::version::{Levels, table_in_range};

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
/// With [`L0Metric::Depth`], this many L0 files count as one level of depth
/// at least, so that L0 cannot grow without bound when its files do not
/// overlap (sequential keys).
pub const L0_FILES_PER_DEPTH: usize = 2;
/// The slowest rate of [`SlowdownPolicy::Rate`].
const MIN_SLOWDOWN_RATE: u64 = 1 << 20;
/// The longest delay of one commit under [`SlowdownPolicy::Rate`].
const MAX_RATE_DELAY: Duration = Duration::from_millis(100);

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
    pub l0_metric: L0Metric,
    pub l0_priority: bool,
    pub l0_partial: bool,
    pub l0_trivial_move: bool,
    pub flush_split: bool,
    pub max_compaction_bytes: u64,
    pub grandparent_overlap_bytes: u64,
    pub heap_merge_threshold: usize,
    pub max_subcompactions: usize,
    pub close_subcompactions: usize,
    /// New tables honour `bytes_per_sync`. Off for the compactions of a
    /// close: nothing else is writing, so the intermediate syncs only slow
    /// the output down.
    pub intermediate_syncs: bool,
}

impl Tuning {
    pub fn new(cfg: &Config) -> Self {
        let c = &cfg.compaction;
        let write_buffer = cfg.write_buffer_bytes.max(MIN_WRITE_BUFFER);
        let wb = write_buffer as u64;
        let target_file_size = c.target_file_size.unwrap_or(wb);
        Tuning {
            write_buffer,
            target_file_size,
            l1_max_bytes: c.level_base_bytes.unwrap_or_else(|| (L0_COMPACTION_TRIGGER as u64).saturating_mul(wb)),
            l0_trigger: c.l0_trigger,
            l0_slowdown: c.l0_slowdown,
            l0_stop: c.l0_stop,
            level_multiplier: c.level_multiplier,
            slowdown: c.slowdown,
            l0_metric: c.l0_metric,
            l0_priority: c.l0_priority,
            l0_partial: c.l0_partial,
            l0_trivial_move: c.l0_trivial_move,
            flush_split: c.flush_split,
            max_compaction_bytes: c.max_compaction_bytes.unwrap_or_else(|| target_file_size.saturating_mul(25)),
            grandparent_overlap_bytes: c
                .grandparent_overlap_bytes
                .unwrap_or_else(|| target_file_size.saturating_mul(10)),
            heap_merge_threshold: c.heap_merge_threshold,
            max_subcompactions: c.max_subcompactions,
            // Inline mode promises no threads of its own (`Background::Inline`).
            close_subcompactions: match cfg.test.background {
                Background::Inline => c.max_subcompactions,
                Background::Threads => c.close_subcompactions,
            },
            intermediate_syncs: true,
        }
    }

    /// The L0 pressure that `l0_trigger`, `l0_slowdown` and `l0_stop` are
    /// compared with.
    pub fn l0_metric(&self, files: &[Arc<Table>]) -> usize {
        match self.l0_metric {
            L0Metric::Files => files.len(),
            L0Metric::Depth => l0_depth(files).max(files.len().div_ceil(L0_FILES_PER_DEPTH)),
        }
    }

    pub fn level_max_bytes(&self, level: usize) -> u64 {
        debug_assert!(level >= 1);
        self.l1_max_bytes.saturating_mul(self.level_multiplier.saturating_pow(level as u32 - 1))
    }
}

/// The rate of [`SlowdownPolicy::Rate`]: how fast writers may go while L0 is
/// at or above `l0_slowdown`. Owned by the writer, which is single.
#[derive(Debug)]
pub struct RateController {
    initial: u64,
    rate: u64,
    last_metric: usize,
}

impl RateController {
    pub fn new(initial_bytes_per_sec: u64) -> Self {
        RateController { initial: initial_bytes_per_sec, rate: initial_bytes_per_sec, last_metric: 0 }
    }

    /// Records the L0 metric a commit found. The rate halves (down to 1 MiB/s,
    /// or the initial rate if lower) while the metric grows at or above
    /// `slowdown`, and doubles back up to the initial rate when it falls or
    /// is below `slowdown`.
    pub fn observe(&mut self, metric: usize, slowdown: usize) {
        let floor = MIN_SLOWDOWN_RATE.min(self.initial);
        if metric >= slowdown && metric > self.last_metric {
            self.rate = (self.rate / 2).max(floor);
        } else if metric < slowdown || metric < self.last_metric {
            self.rate = self.rate.saturating_mul(2).min(self.initial);
        }
        self.last_metric = metric;
    }

    /// How long a commit of `bytes` bytes waits at the current rate.
    pub fn delay(&self, bytes: u64) -> Duration {
        let micros = (u128::from(bytes) * 1_000_000 / u128::from(self.rate.max(1))).min(MAX_RATE_DELAY.as_micros());
        Duration::from_micros(micros as u64)
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
    /// The single input moves to the next level without being rewritten.
    trivial: bool,
    /// Chosen by `compact_range`, not by the score of a level.
    directed: bool,
}

impl Plan {
    pub fn is_trivial_move(&self) -> bool {
        self.trivial
    }

    pub fn largest_input_key(&self) -> &[u8] {
        self.inputs.iter().map(|t| t.largest.as_slice()).max().unwrap_or_default()
    }

    pub fn all_inputs(&self) -> impl Iterator<Item = &Arc<Table>> {
        self.inputs.iter().chain(&self.next_inputs)
    }

    /// Bytes the compaction reads and writes, taking the output to be as
    /// large as the input (none for a trivial move).
    pub fn io_bytes(&self) -> u64 {
        if self.trivial { 0 } else { 2 * self.all_inputs().map(|t| t.size).sum::<u64>() }
    }

    /// The keys the compaction reads, and the level it writes to.
    fn footprint(&self) -> Footprint {
        let lo = self.all_inputs().map(|t| t.smallest.as_slice()).min().unwrap_or_default();
        let hi = self.all_inputs().map(|t| t.largest.as_slice()).max().unwrap_or_default();
        Footprint { level: self.level + 1, lo: lo.to_vec(), hi: hi.to_vec() }
    }
}

fn level_bytes(level: &[Arc<Table>]) -> u64 {
    level.iter().map(|t| t.size).sum()
}

/// The files of `level` (sorted, non-overlapping) whose key range
/// intersects `[lo, hi]`.
fn overlapping<'a>(level: &'a [Arc<Table>], lo: &[u8], hi: &[u8]) -> &'a [Arc<Table>] {
    let start = level.partition_point(|t| t.ends_before(lo));
    let end = level.partition_point(|t| t.smallest.as_slice() <= hi);
    &level[start..end.max(start)]
}

/// The compactions in flight (several with `max_background_compactions > 1`),
/// as far as choosing the next one is concerned. Guarded by the engine's
/// state lock: a compaction is claimed in the same critical section that
/// picks it, so two threads never choose conflicting work.
///
/// A candidate conflicts with the ones in flight when
/// * it shares an input table with one of them, or
/// * it writes to the same level as one of them and the key ranges of what
///   they read overlap: their outputs could overlap each other.
///
/// With both rules, whatever a compaction in flight reads or writes in a
/// level is disjoint, in keys, from what any other one touches there, which
/// is also why dropping a tombstone because "no deeper level holds the key"
/// stays safe: the deeper files that matter are inputs of nobody else.
#[derive(Default)]
pub struct Busy {
    tables: HashSet<u64>,
    outputs: Vec<Footprint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Footprint {
    /// The level the compaction writes to.
    level: usize,
    lo: Vec<u8>,
    hi: Vec<u8>,
}

/// What [`Busy::claim`] took, to give back with [`Busy::release`].
pub struct Claim {
    numbers: Vec<u64>,
    footprint: Footprint,
}

impl Busy {
    fn conflicts(&self, plan: &Plan) -> bool {
        if plan.all_inputs().any(|t| self.tables.contains(&t.number)) {
            return true;
        }
        let f = plan.footprint();
        self.outputs.iter().any(|o| o.level == f.level && o.lo <= f.hi && f.lo <= o.hi)
    }

    /// Records `plan` as in flight. It must not conflict.
    pub fn claim(&mut self, plan: &Plan) -> Claim {
        debug_assert!(!self.conflicts(plan));
        let numbers: Vec<u64> = plan.all_inputs().map(|t| t.number).collect();
        self.tables.extend(numbers.iter().copied());
        let footprint = plan.footprint();
        self.outputs.push(footprint.clone());
        Claim { numbers, footprint }
    }

    pub fn release(&mut self, claim: &Claim) {
        for n in &claim.numbers {
            self.tables.remove(n);
        }
        if let Some(i) = self.outputs.iter().position(|o| *o == claim.footprint) {
            self.outputs.swap_remove(i);
        }
    }
}

/// Picks the most urgent compaction that does not conflict with the ones in
/// `busy`, if any level is over its target. `pointers[n]` is the largest key
/// of the previous compaction out of Ln.
pub fn pick(levels: &Levels, tuning: &Tuning, pointers: &[Vec<u8>], busy: &Busy) -> Option<Plan> {
    let metric = tuning.l0_metric(&levels[0]);
    let l0 = metric as f64 / tuning.l0_trigger as f64;
    let mut due: Vec<(f64, usize)> = Vec::new();
    if l0 >= 1.0 {
        due.push((if tuning.l0_priority && metric >= tuning.l0_slowdown { f64::INFINITY } else { l0 }, 0));
    }
    for (n, files) in levels.iter().enumerate().take(NUM_LEVELS - 1).skip(1) {
        let score = level_bytes(files) as f64 / tuning.level_max_bytes(n) as f64;
        if score >= 1.0 {
            due.push((score, n));
        }
    }
    // Highest score first; on a tie, the shallower level.
    due.sort_by(|a, b| b.0.total_cmp(&a.0));
    due.into_iter().find_map(|(_, level)| pick_level(levels, tuning, pointers, busy, level))
}

/// The compaction out of `level` that follows the round-robin position, or
/// the first one after it that does not conflict.
fn pick_level(levels: &Levels, tuning: &Tuning, pointers: &[Vec<u8>], busy: &Busy, level: usize) -> Option<Plan> {
    if level == 0 {
        return pick_l0(levels, tuning, busy);
    }
    let files = &levels[level];
    let after = files.iter().position(|t| t.largest > pointers[level]).unwrap_or(0);
    (0..files.len()).find_map(|k| {
        let input = files[(after + k) % files.len()].clone();
        let next_inputs = overlapping(&levels[level + 1], &input.smallest, &input.largest).to_vec();
        let trivial = next_inputs.is_empty() && !input.has_fragments();
        let plan = Plan {
            level,
            inputs: vec![input],
            next_inputs,
            deeper: levels[level + 2..].to_vec(),
            trivial,
            directed: false,
        };
        (!busy.conflicts(&plan)).then_some(plan)
    })
}

/// The L0 compaction, whether or not L0 is over its trigger; `None` if L0 is
/// empty or the compaction conflicts with one in `busy`.
///
/// L0 is newest first, and the compaction takes the OLDEST files only (all
/// of them without `l0_partial`): what stays is newer than everything that
/// moves down, so reads keep finding the newest version first.
pub fn pick_l0(levels: &Levels, tuning: &Tuning, busy: &Busy) -> Option<Plan> {
    pick_l0_files(levels, tuning, busy, false)
}

/// The mandatory compaction of a close: like [`pick_l0`], but with
/// `l0_partial` it takes no more of the oldest files than it needs to leave
/// the L0 metric below `l0_trigger`. When every L0 file spans all the keys
/// (a random-order load) each one moves all of L1, so the files it leaves
/// for the next session are bytes the close neither rewrites nor deletes.
pub fn pick_l0_floor(levels: &Levels, tuning: &Tuning) -> Option<Plan> {
    pick_l0_files(levels, tuning, &Busy::default(), true)
}

fn pick_l0_files(levels: &Levels, tuning: &Tuning, busy: &Busy, floor: bool) -> Option<Plan> {
    let l0 = &levels[0];
    let oldest = l0.last()?;
    let deeper = levels[2..].to_vec();
    let plan = if tuning.l0_trivial_move
        && !oldest.has_fragments()
        && overlapping(&levels[1], &oldest.smallest, &oldest.largest).is_empty()
    {
        Plan { level: 0, inputs: vec![oldest.clone()], next_inputs: Vec::new(), deeper, trivial: true, directed: false }
    } else {
        let mut take = l0.len();
        if tuning.l0_partial {
            take = 0;
            let (mut lo, mut hi, mut bytes) = (oldest.smallest.as_slice(), oldest.largest.as_slice(), 0u64);
            for t in l0.iter().rev() {
                if floor && take > 0 && tuning.l0_metric(&l0[..l0.len() - take]) < tuning.l0_trigger {
                    break;
                }
                let (nlo, nhi) = (lo.min(t.smallest.as_slice()), hi.max(t.largest.as_slice()));
                let total = bytes + t.size + level_bytes(overlapping(&levels[1], nlo, nhi));
                if take > 0 && total > tuning.max_compaction_bytes {
                    break;
                }
                (lo, hi, bytes, take) = (nlo, nhi, bytes + t.size, take + 1);
            }
        }
        let inputs = l0[l0.len() - take..].to_vec();
        let lo = inputs.iter().map(|t| t.smallest.as_slice()).min()?;
        let hi = inputs.iter().map(|t| t.largest.as_slice()).max()?;
        let next_inputs = overlapping(&levels[1], lo, hi).to_vec();
        Plan { level: 0, inputs, next_inputs, deeper, trivial: false, directed: false }
    };
    (!busy.conflicts(&plan)).then_some(plan)
}

/// What [`pick_range`] found.
pub enum RangeStep {
    /// No file above the last level overlaps the range: it is at the bottom.
    Done,
    /// The next step conflicts with a compaction in flight; try again when
    /// it ends.
    Blocked,
    Plan(Plan),
}

/// The next step of a directed compaction (`compact_range`) of the keys
/// between the bounds: the shallowest level above the last that has files
/// overlapping the range, compacted into the next level. Repeating it
/// pushes everything that overlaps the range to the last level, and the
/// tombstones, which the merges drop when nothing deeper overlaps them, out
/// of the files.
///
/// From L0 it takes the oldest suffix that holds every overlapping file, as
/// every L0 compaction must (what stays is newer than what moves down). A
/// file that overlaps nothing below it moves down without being rewritten
/// only if it holds no tombstone at all: the rewrite is what drops them.
pub fn pick_range(levels: &Levels, busy: &Busy, start: Bound<&[u8]>, end: Bound<&[u8]>) -> RangeStep {
    for level in 0..NUM_LEVELS - 1 {
        let files = &levels[level];
        let inputs: Vec<Arc<Table>> = if level == 0 {
            match files.iter().position(|t| table_in_range(t, start, end)) {
                Some(newest) => files[newest..].to_vec(),
                None => continue,
            }
        } else {
            files.iter().filter(|t| table_in_range(t, start, end)).cloned().collect()
        };
        let (Some(lo), Some(hi)) =
            (inputs.iter().map(|t| t.smallest.as_slice()).min(), inputs.iter().map(|t| t.largest.as_slice()).max())
        else {
            continue;
        };
        let next_inputs = overlapping(&levels[level + 1], lo, hi).to_vec();
        let trivial =
            next_inputs.is_empty() && inputs.len() == 1 && !inputs[0].has_fragments() && inputs[0].point_deletes() == 0;
        let plan = Plan { level, inputs, next_inputs, deeper: levels[level + 2..].to_vec(), trivial, directed: true };
        return if busy.conflicts(&plan) { RangeStep::Blocked } else { RangeStep::Plan(plan) };
    }
    RangeStep::Done
}

/// Bytes that leveled compaction still has to move: for every level from L1
/// to the second-to-last (the last one is never compacted), the bytes above
/// its target, plus all of L0 once its metric reaches `l0_trigger`.
pub fn pending_bytes(levels: &Levels, tuning: &Tuning) -> u64 {
    let mut pending = 0u64;
    if tuning.l0_metric(&levels[0]) >= tuning.l0_trigger {
        pending = level_bytes(&levels[0]);
    }
    for (n, files) in levels.iter().enumerate().take(NUM_LEVELS - 1).skip(1) {
        pending = pending.saturating_add(level_bytes(files).saturating_sub(tuning.level_max_bytes(n)));
    }
    pending
}

/// The largest number of L0 files whose key ranges contain one same key.
pub fn l0_depth(files: &[Arc<Table>]) -> usize {
    // Closed ranges, so at equal keys the starts are counted before the
    // ends; but a range that ends exclusively does not hold its last key, so
    // its end comes first.
    const END_EXCLUSIVE: u8 = 0;
    const START: u8 = 1;
    const END: u8 = 2;
    let mut events: Vec<(&[u8], u8)> = Vec::with_capacity(files.len() * 2);
    for t in files {
        events.push((t.smallest.as_slice(), START));
        events.push((t.largest.as_slice(), if t.largest_exclusive { END_EXCLUSIVE } else { END }));
    }
    events.sort_unstable();
    let (mut open, mut depth) = (0usize, 0usize);
    for (_, kind) in events {
        if kind == START {
            open += 1;
            depth = depth.max(open);
        } else {
            open -= 1;
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
            while *cur < files.len() && files[*cur].ends_before(key) {
                *cur += 1;
            }
            if *cur < files.len() && files[*cur].smallest.as_slice() <= key {
                return true;
            }
        }
        false
    }

    /// Whether a file below the output level overlaps `[start, end)`, so that
    /// something there may still need the range tombstone. Ranges must be
    /// presented in increasing order of `start`.
    fn range_may_overlap(&mut self, start: &[u8], end: &[u8]) -> bool {
        for (files, cur) in self.levels.iter().zip(&mut self.cursors) {
            while *cur < files.len() && files[*cur].ends_before(start) {
                *cur += 1;
            }
            if *cur < files.len() && files[*cur].smallest.as_slice() < end {
                return true;
            }
        }
        false
    }
}

/// Decides where a compaction output is cut so that it does not overlap too
/// much of the level below the output level (the "grandparents"): a later
/// compaction of that output would otherwise have to rewrite all of them.
/// Keys are presented in increasing order.
struct GrandparentCut<'a> {
    files: &'a [Arc<Table>],
    limit: u64,
    next: usize,
    /// Bytes of grandparent files passed since the last cut, counted once an
    /// output has started.
    overlapped: u64,
    started: bool,
}

impl<'a> GrandparentCut<'a> {
    fn new(files: &'a [Arc<Table>], limit: u64) -> Self {
        GrandparentCut { files, limit, next: 0, overlapped: 0, started: false }
    }

    /// Whether the output being written must end before `key`.
    fn cut_before(&mut self, key: &[u8]) -> bool {
        while self.next < self.files.len() && self.files[self.next].ends_before(key) {
            if self.started {
                self.overlapped = self.overlapped.saturating_add(self.files[self.next].size);
            }
            self.next += 1;
        }
        self.started = true;
        if self.overlapped > self.limit {
            self.overlapped = 0;
            return true;
        }
        false
    }
}

/// A `[lo, hi)` key range, `None` for "no bound".
type KeyRange<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);

/// The key ranges an L0 compaction is split into,`[lo, hi)` with `None` for
/// "no bound": at most `max_subcompactions`, cut at the smallest keys of L1
/// input files so that no L1 file is shared between two parts, with about
/// equal L1 bytes each. One part (all keys) when the plan is not an L0
/// compaction or has fewer than two L1 files.
fn subranges(plan: &Plan, max_subcompactions: usize) -> Vec<KeyRange<'_>> {
    let files = &plan.next_inputs;
    let parts = max_subcompactions.min(files.len());
    if plan.level != 0 || parts < 2 {
        return vec![(None, None)];
    }
    let total: u64 = files.iter().map(|t| t.size).sum();
    let mut cuts: Vec<&[u8]> = Vec::with_capacity(parts - 1);
    let (mut seen, mut next_part) = (0u64, 1u64);
    for (i, t) in files.iter().enumerate() {
        // A cut before file `i` once the files before it hold the share of
        // the parts so far; never before the first file.
        if i > 0 && seen * parts as u64 >= next_part * total && cuts.len() < parts - 1 {
            cuts.push(t.smallest.as_slice());
            next_part += 1;
        }
        seen += t.size;
    }
    let mut ranges = Vec::with_capacity(cuts.len() + 1);
    let mut lo = None;
    for cut in cuts {
        ranges.push((lo, Some(cut)));
        lo = Some(cut);
    }
    ranges.push((lo, None));
    ranges
}

/// Merges the plan's inputs into new, synced tables (not yet referenced by
/// the MANIFEST). Returns `None` if `abort` was raised; partial outputs are
/// removed in that case and on error.
///
/// With `max_subcompactions > 1` an L0 compaction runs as several merges of
/// disjoint key ranges at once (see [`subranges`]); their outputs are
/// concatenated in key order and installed together by the caller, with one
/// MANIFEST write.
pub fn run(
    ctx: &Arc<TableCtx>,
    plan: &Plan,
    tuning: &Tuning,
    next_file: &AtomicU64,
    abort: &AtomicBool,
    stats: &Stats,
) -> Result<Option<Vec<Arc<Table>>>> {
    let ranges = subranges(plan, tuning.max_subcompactions);
    let parts: Vec<Result<Option<Vec<Table>>>> = if let [range] = ranges[..] {
        vec![run_range(ctx, plan, tuning, next_file, abort, stats, range)]
    } else {
        bump(&stats.subcompactions, ranges.len() as u64);
        std::thread::scope(|scope| {
            let handles: Vec<_> = ranges
                .iter()
                .map(|&range| {
                    scope.spawn(move || {
                        let _phase = kerno_core::probe::phase("compact");
                        run_range(ctx, plan, tuning, next_file, abort, stats, range)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|_| Err(Error::Io(std::io::Error::other("subcompaction panicked")))))
                .collect()
        })
    };
    // All or nothing: one failed or abandoned part removes the others'
    // outputs too.
    let mut outputs: Vec<Table> = Vec::new();
    let mut failure: Option<Result<Option<Vec<Table>>>> = None;
    for part in parts {
        match part {
            Ok(Some(tables)) => outputs.extend(tables),
            other => {
                failure.get_or_insert(other);
            }
        }
    }
    match failure {
        None => Ok(Some(outputs.into_iter().map(Arc::new).collect())),
        Some(failed) => {
            for t in &outputs {
                t.mark_obsolete(); // deleted on drop
            }
            failed.map(|_| None)
        }
    }
}

/// One merge of the plan's inputs restricted to the keys in `[lo, hi)`.
fn run_range(
    ctx: &Arc<TableCtx>,
    plan: &Plan,
    tuning: &Tuning,
    next_file: &AtomicU64,
    abort: &AtomicBool,
    stats: &Stats,
    range: (Option<&[u8]>, Option<&[u8]>),
) -> Result<Option<Vec<Table>>> {
    let mut outputs: Vec<Table> = Vec::new();
    let mut builder: Option<TableBuilder> = None;
    let result = merge_into(ctx, plan, tuning, next_file, abort, stats, range, &mut outputs, &mut builder);
    match result {
        Ok(true) => Ok(Some(outputs)),
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
    (lo, hi): (Option<&[u8]>, Option<&[u8]>),
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
    let mut it = MergeIter::with_heap_threshold(children, tuning.heap_merge_threshold);
    let mut deeper = DeeperLevels::new(&plan.deeper);
    let mut grandparents =
        GrandparentCut::new(plan.deeper.first().map_or(&[], Vec::as_slice), tuning.grandparent_overlap_bytes);
    // The range tombstones of the inputs, the highest sequence over each
    // part: they decide which points go, and which fragments the outputs keep.
    let all = rangedel::aggregate(plan.all_inputs().map(|t| t.fragments()));
    let mut cover = CoverCursor::new(vec![Cow::Borrowed(&all[..])]);
    let kept = keep_fragments(&all, plan, lo, hi, stats);
    let mut splitter = FragmentSplitter::new(kept);
    match lo {
        Some(lo) => {
            it.seek(lo)?;
            cover.seek(lo);
        }
        None => {
            it.seek_to_first()?;
            cover.seek(&[]);
        }
    }
    let mut n = 0u64;
    // The output being written has reached its target size: it ends before
    // the next key, so that the fragments can be cut there.
    let mut size_reached = false;
    while it.valid() && hi.is_none_or(|hi| it.key() < hi) {
        n += 1;
        if n.is_multiple_of(512) && abort.load(Ordering::Relaxed) {
            return Ok(false);
        }
        if cover.covering(it.key()) > it.seq() {
            // Hidden by a newer tombstone: the point is garbage now, and the
            // fragment (if kept) still hides what is older, below.
            bump(&stats.covered_points_dropped, 1);
            it.next()?;
            continue;
        }
        if it.value().is_none() && !deeper.may_contain(it.key()) {
            bump(&stats.tombstones_dropped, 1);
            it.next()?;
            continue;
        }
        let cut = grandparents.cut_before(it.key()) || size_reached;
        if cut && let Some(b) = builder.take() {
            outputs.push(b.finish_with(&splitter.take_below(Some(it.key())))?);
        }
        if builder.is_none() {
            let b = TableBuilder::create(ctx, next_file.fetch_add(1, Ordering::SeqCst), output_reason(plan))?;
            *builder = Some(if tuning.intermediate_syncs { b } else { b.without_intermediate_syncs() });
        }
        let b = builder.as_mut().expect("just created");
        b.add(it.key(), it.seq(), it.value())?;
        size_reached = b.estimated_size() >= tuning.target_file_size;
        it.next()?;
    }
    let rest = splitter.take_below(None);
    if builder.is_none() && !rest.is_empty() {
        // Nothing but tombstones left in this part: a table of only those.
        let b = TableBuilder::create(ctx, next_file.fetch_add(1, Ordering::SeqCst), output_reason(plan))?;
        *builder = Some(if tuning.intermediate_syncs { b } else { b.without_intermediate_syncs() });
    }
    if let Some(b) = builder.take() {
        outputs.push(b.finish_with(&rest)?);
    }
    Ok(true)
}

/// The `creation_reason` of the tables a plan writes.
fn output_reason(plan: &Plan) -> u8 {
    if plan.directed { REASON_DIRECTED } else { REASON_COMPACTION }
}

/// The fragments of `all` (sorted, disjoint) that a merge of the keys in
/// `[lo, hi)` writes: those inside the part, minus the ones no file below the
/// output level overlaps, which have nothing left to hide (SEM-7).
fn keep_fragments(all: &[Fragment], plan: &Plan, lo: Option<&[u8]>, hi: Option<&[u8]>, stats: &Stats) -> Vec<Fragment> {
    let part = rangedel::clip(all.to_vec(), lo, hi);
    let total = part.len();
    let mut deeper = DeeperLevels::new(&plan.deeper);
    let kept: Vec<Fragment> = part.into_iter().filter(|f| deeper.range_may_overlap(&f.start, &f.end)).collect();
    bump(&stats.range_tombstones_dropped, (total - kept.len()) as u64);
    kept
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
    if out.windows(2).any(|w| !w[0].ends_before(&w[1].smallest)) {
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
    use crate::config::CompactionConfig;
    use crate::format::props::REASON_FLUSH;
    use kerno_core::contract::Options;

    /// Round-robin positions that choose nothing.
    static NO_POINTERS: [Vec<u8>; NUM_LEVELS] = [const { Vec::new() }; NUM_LEVELS];

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
        let mut b = TableBuilder::create(ctx, number, REASON_FLUSH).unwrap();
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
    fn busy_keeps_compactions_from_sharing_tables() {
        let ctx = ctx();
        let mut cfg = Config::from(&Options::small_for_tests());
        cfg.compaction.l0_trigger = 100;
        let tuning = Tuning { l1_max_bytes: 1, ..Tuning::new(&cfg) };
        let mut levels: Levels = vec![Vec::new(); NUM_LEVELS];
        levels[1] = vec![table(&ctx, 1, "a", "b"), table(&ctx, 2, "c", "d"), table(&ctx, 3, "e", "f")];
        levels[2] = vec![table(&ctx, 4, "a", "d")];
        let input = |p: &Plan| p.inputs[0].number;

        let mut busy = Busy::default();
        let first = pick(&levels, &tuning, &NO_POINTERS, &busy).unwrap();
        assert_eq!((input(&first), first.next_inputs.len()), (1, 1));
        let claim_first = busy.claim(&first);
        // Files 1 and 2 both need file 4: only 3, which overlaps nothing.
        let second = pick(&levels, &tuning, &NO_POINTERS, &busy).unwrap();
        assert_eq!(input(&second), 3);
        assert!(second.is_trivial_move());
        let claim_second = busy.claim(&second);
        assert!(pick(&levels, &tuning, &NO_POINTERS, &busy).is_none(), "everything is spoken for");
        // Giving a claim back frees what it held.
        busy.release(&claim_first);
        assert_eq!(input(&pick(&levels, &tuning, &NO_POINTERS, &busy).unwrap()), 1);
        busy.release(&claim_second);
        assert!(busy.tables.is_empty() && busy.outputs.is_empty());
    }

    #[test]
    fn l0_waits_while_any_of_its_files_is_in_flight() {
        let ctx = ctx();
        let mut cfg = Config::from(&Options::small_for_tests());
        cfg.compaction = CompactionConfig { l0_trigger: 2, l0_partial: true, ..cfg.compaction };
        cfg.compaction.max_compaction_bytes = Some(1); // one file at a time
        let tuning = Tuning::new(&cfg);
        let mut levels: Levels = vec![Vec::new(); NUM_LEVELS];
        // Newest first; the oldest file is 1.
        levels[0] = vec![table(&ctx, 3, "a", "b"), table(&ctx, 2, "a", "b"), table(&ctx, 1, "a", "b")];
        let mut busy = Busy::default();
        let plan = pick(&levels, &tuning, &NO_POINTERS, &busy).unwrap();
        assert_eq!(plan.inputs.iter().map(|t| t.number).collect::<Vec<_>>(), [1]);
        let claim = busy.claim(&plan);
        // The next L0 compaction would start from the same oldest file.
        assert!(pick(&levels, &tuning, &NO_POINTERS, &busy).is_none());
        busy.release(&claim);
        assert!(pick(&levels, &tuning, &NO_POINTERS, &busy).is_some());
    }

    #[test]
    fn rate_halves_while_the_metric_grows_and_recovers_when_it_falls() {
        let mib = 1u64 << 20;
        let mut r = RateController::new(16 * mib);
        // Below the slowdown limit nothing changes.
        r.observe(3, 8);
        assert_eq!(r.rate, 16 * mib);
        // Growing at or above it: halves, down to 1 MiB/s.
        let mut seen = Vec::new();
        for metric in [8, 9, 9, 10, 11, 12, 13, 14] {
            r.observe(metric, 8);
            seen.push(r.rate / mib);
        }
        assert_eq!(seen, [8, 4, 4, 2, 1, 1, 1, 1]);
        // Falling, or below the limit: doubles back up to the initial rate.
        let mut seen = Vec::new();
        for metric in [13, 12, 7, 7, 7, 7, 7, 7] {
            r.observe(metric, 8);
            seen.push(r.rate / mib);
        }
        assert_eq!(seen, [2, 4, 8, 16, 16, 16, 16, 16]);
        // A rate below 1 MiB/s is never raised above, nor lowered under, itself.
        let mut slow = RateController::new(1000);
        slow.observe(9, 8);
        slow.observe(10, 8);
        assert_eq!(slow.rate, 1000);
    }

    #[test]
    fn rate_delay_is_bytes_over_rate_and_bounded() {
        let r = RateController::new(1 << 20);
        assert_eq!(r.delay(0), Duration::ZERO);
        assert_eq!(r.delay(1 << 10), Duration::from_micros(976));
        assert_eq!(r.delay(1 << 40), MAX_RATE_DELAY);
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
