//! `assoc_bench`: performance of the associative query (E7-05, acta §6.3).
//!
//! The reference path ([`UnitSnapshot::reference_assoc_query`]) and the
//! indexed path ([`UnitSnapshot::indexed_assoc_query`]) run the 64 queries of
//! the performance scale of the frozen generator (20 000 units, 3 000 change
//! sets, `KernoDb` on the real file system), at depths 1, 2 and 3. Depth 2 is
//! the one with a threshold: the indexed path's p50 must not be worse than the
//! reference path's. The rest is informative.
//!
//! Protocol, fixed before the first measurement:
//!
//! * every pair runs the two paths on the same query and the same snapshot,
//!   one after the other, and the order alternates pair by pair;
//! * the pairs run in rounds (repetitions of the 64 queries); the first
//!   `WARM_REPS` rounds of each run are warm-up and not counted. The committed
//!   runs used `--reps 12`, so 4 of 12 rounds (a third), not the 10 % that the
//!   default of 40 rounds gives; the write-up records this;
//! * each run is one pass over the rounds at one depth; `--runs` runs follow
//!   each other in one process, and the whole command is repeated in separate
//!   processes so that every run is reported;
//! * each timed answer is compared with the other path's after both are timed,
//!   and the run stops if they differ;
//! * the load average before and after each run is printed (the machine has 4
//!   CPUs and may be shared).
//!
//! The verdict is printed with its numbers: the depth-2 threshold is met only
//! if the indexed p50 is no worse than the reference p50 in every run. A run
//! where it is worse is a run where it is not met; the runs are never
//! averaged away.
//!
//! The translation of the queries is the one of the quality measurement
//! (`tests/harness/assoc.rs`, `corpus_query`), shared through `#[path]`.
//!
//! ```text
//! cargo run --release -p kerno-units --example assoc_bench -- --runs 5 --dir <scratch>
//! ```
//!
//! Options: `--runs <n>` (default 5), `--reps <n>` rounds per run (default 40),
//! `--dir <path>` (default: the system temporary directory).

// The frozen-set helpers are shared with the tests; this example uses only
// the translation.
#[path = "../tests/harness/assoc.rs"]
#[allow(dead_code)]
mod assoc;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use kerno_core::contract::Options;
use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, Scale, Story};
use kerno_units::*;

type Res<T> = Result<T, Box<dyn std::error::Error>>;
type Units = UnitDb<KernoDb>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const DEPTHS: [u32; 3] = [1, 2, 3];
/// The depth whose p50 has a threshold (acta §6.3).
const THRESHOLD_DEPTH: u32 = 2;
/// Rounds of the 64 queries per run, and the warm-up rounds (10 % of them).
const REPS: usize = 40;
const WARM_REPS: usize = 4;

/// Removes its directory when dropped.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The corpus loaded into a database on the real file system, one change set
/// per step, as in the quality measurement. Returns the id of each index.
fn load(dir: &std::path::Path, corpus: &assoc_eval::Corpus) -> Res<(Units, Vec<UnitId>)> {
    let mut cfg = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    cfg.engine = Options::default();
    let db = Units::open(StdVfs::shared(), dir, &cfg)?;
    let mut ids = Vec::with_capacity(corpus.units.len());
    let (mut next_unit, mut next_rel) = (0, 0);
    for change in &corpus.changes {
        let meta = ChangeMeta {
            actor: Some(change.actor.clone().into_bytes()),
            source: Some(change.source.clone().into_bytes()),
            ..ChangeMeta::default()
        };
        let mut cs = db.begin_changeset(meta)?;
        while let Some(u) = corpus.units.get(next_unit).filter(|u| u.step == change.step) {
            let new = NewUnit {
                unit_type: u.unit_type.clone(),
                schema_version: 1,
                metadata: Vec::new(),
                payload: u.label.clone().into_bytes(),
            };
            ids.push(cs.create(new)?);
            next_unit += 1;
        }
        while let Some(r) = corpus.relations.get(next_rel).filter(|r| r.step == change.step) {
            let (from, to) = (ids[r.from as usize], ids[r.to as usize]);
            cs.create_relation(from, to, &r.relation_type, assoc::story_provenance(&r.provenance))?;
            next_rel += 1;
        }
        let info = cs.commit()?;
        if info.commit_seq.0 != change.step {
            return Err(format!("step {} committed as sequence {}", change.step, info.commit_seq.0).into());
        }
    }
    Ok((db, ids))
}

/// Latencies in nanoseconds and sizes of one depth over one run.
#[derive(Default)]
struct Series {
    reference: Vec<u64>,
    indexed: Vec<u64>,
    visited: u64,
    hits: u64,
    answers: u64,
}

/// `(p50, p99)` in milliseconds, nearest rank.
fn percentiles(samples: &mut [u64]) -> (f64, f64) {
    samples.sort_unstable();
    let at = |q: f64| {
        let rank = ((q * samples.len() as f64).ceil() as usize).clamp(1, samples.len());
        samples[rank - 1] as f64 / 1e6
    };
    (at(0.50), at(0.99))
}

/// One run at one depth: the rounds, each pair timed, the order alternating.
fn run_depth(snap: &UnitSnapshot<KernoDb>, base: &[AssocQuery], depth: u32, reps: usize) -> Res<Series> {
    let queries: Vec<AssocQuery> = base.iter().map(|q| AssocQuery { max_depth: depth, ..q.clone() }).collect();
    let mut series = Series::default();
    for rep in 0..reps {
        for (i, q) in queries.iter().enumerate() {
            let (reference, indexed, t_ref, t_idx) = if (rep + i) % 2 == 0 {
                let t = Instant::now();
                let r = snap.reference_assoc_query(q)?;
                let t_r = t.elapsed().as_nanos() as u64;
                let t = Instant::now();
                let x = snap.indexed_assoc_query(q)?;
                (r, x, t_r, t.elapsed().as_nanos() as u64)
            } else {
                let t = Instant::now();
                let x = snap.indexed_assoc_query(q)?;
                let t_x = t.elapsed().as_nanos() as u64;
                let t = Instant::now();
                let r = snap.reference_assoc_query(q)?;
                (r, x, t.elapsed().as_nanos() as u64, t_x)
            };
            if reference != indexed {
                return Err(format!("depth {depth}, query {i}: the indexed path differs from the reference").into());
            }
            if rep >= WARM_REPS {
                series.reference.push(t_ref);
                series.indexed.push(t_idx);
                series.visited += reference.visited;
                series.hits += reference.hits.len() as u64;
                series.answers += 1;
            }
        }
    }
    Ok(series)
}

/// The load average (1, 5 and 15 minutes), or `n/a` where it is not readable.
fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|_| "n/a".to_owned())
}

/// What one run found at one depth, for the verdict.
struct Summary {
    run: usize,
    depth: u32,
    p50_ref: f64,
    p50_idx: f64,
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let runs: usize = arg("--runs").map_or(Ok(5), |s| s.parse())?;
    let reps: usize = arg("--reps").map_or(Ok(REPS), |s| s.parse())?;
    if reps <= WARM_REPS {
        return Err("--reps must be above the warm-up rounds".into());
    }
    let base = arg("--dir").map_or_else(std::env::temp_dir, PathBuf::from);

    let story = Story::generate(Scale::Performance, assoc_eval::SEED);
    let dir = TempDir(base.join(format!("kerno-assoc-bench-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0)?;
    let t = Instant::now();
    let (db, ids) = load(&dir.0, &story.corpus)?;
    println!(
        "# load: {} units, {} relations, {} change sets, {} queries; load time {:.1} s",
        story.corpus.units.len(),
        story.corpus.relations.len(),
        story.corpus.changes.len(),
        story.queries.queries.len(),
        t.elapsed().as_secs_f64()
    );
    let base_queries: Vec<AssocQuery> = story.queries.queries.iter().map(|q| assoc::corpus_query(&ids, q)).collect();

    let snap = db.snapshot()?;
    // An untimed pass first: the caches are filled the same way for every run.
    for q in &base_queries {
        for d in DEPTHS {
            let q = AssocQuery { max_depth: d, ..q.clone() };
            if snap.reference_assoc_query(&q)? != snap.indexed_assoc_query(&q)? {
                return Err("the indexed path differs from the reference".into());
            }
        }
    }

    println!("run,depth,path,answers,p50_ms,p99_ms,mean_visited,mean_hits,loadavg_before,loadavg_after");
    let mut summaries: Vec<Summary> = Vec::new();
    for run in 1..=runs {
        for depth in DEPTHS {
            let before = loadavg();
            let mut s = run_depth(&snap, &base_queries, depth, reps)?;
            let after = loadavg();
            let (r50, r99) = percentiles(&mut s.reference);
            let (i50, i99) = percentiles(&mut s.indexed);
            let n = s.answers as f64;
            let (visited, hits) = (s.visited as f64 / n, s.hits as f64 / n);
            println!("{run},{depth},reference,{},{r50:.4},{r99:.4},{visited:.1},{hits:.1},{before},{after}", s.answers);
            println!("{run},{depth},indexed,{},{i50:.4},{i99:.4},{visited:.1},{hits:.1},{before},{after}", s.answers);
            println!("# run {run} depth {depth}: indexed/reference p50 = {:.3}, p99 = {:.3}", i50 / r50, i99 / r99);
            summaries.push(Summary { run, depth, p50_ref: r50, p50_idx: i50 });
        }
    }

    let worse: Vec<&Summary> =
        summaries.iter().filter(|s| s.depth == THRESHOLD_DEPTH && s.p50_idx > s.p50_ref).collect();
    let runs_at_threshold = summaries.iter().filter(|s| s.depth == THRESHOLD_DEPTH).count();
    println!("# threshold (acta §6.3): depth {THRESHOLD_DEPTH}, indexed p50 not worse than reference p50 in every run");
    for s in summaries.iter().filter(|s| s.depth == THRESHOLD_DEPTH) {
        println!(
            "#   run {}: indexed p50 {:.4} ms, reference p50 {:.4} ms -> {}",
            s.run,
            s.p50_idx,
            s.p50_ref,
            if s.p50_idx <= s.p50_ref { "not worse" } else { "worse" }
        );
    }
    println!(
        "# depth {THRESHOLD_DEPTH}: {} of {runs_at_threshold} runs worse -> {}",
        worse.len(),
        if worse.is_empty() { "met" } else { "not met" }
    );
    db.close()?;
    Ok(())
}
