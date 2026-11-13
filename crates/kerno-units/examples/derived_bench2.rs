//! `derived_bench2`: the two ratios of time of E8-10 (E9-20), the same P1 and P2 of Stage 8 (acta 8 §6.3),
//! at 20 000 units, `KernoDb` on the real file system. There is no absolute
//! latency threshold (decision D9): each threshold is a ratio of two p50 taken
//! in the same session.
//!
//! Stage 9 variant: the same story generator and protocol, with the seed of the second
//! frozen set (`derived_eval2::SEED`); the corpus is generated, not frozen. No new threshold.
//!
//! * **P1**: writing a source together with its derived descriptor and vector
//!   (one change set: create a unit, persist its vector) against writing the
//!   same kind of source without a descriptor (one change set: create a unit).
//!   Threshold: p50 with / p50 without <= 1.50.
//! * **P2**: the exact vector search over the vectors served by the reopened
//!   database against the same search over the same vectors held in memory.
//!   Threshold: p50 persisted / p50 in memory <= 1.10.
//!
//! The corpus is the `Scale::Performance` story of the same generator and seed
//! as the frozen quality set (it is generated, not frozen), loaded with one
//! persisted vector per unit, closed and reopened. What a reopened database
//! serves is read with `DerivedDb::vector`; the in-memory side is built from
//! the generator's own vectors. Both `VectorSet`s are checked to answer every
//! query identically before anything is timed.
//!
//! Protocol, the same repetitions as Stage 7 (`assoc_bench`): pairs run one
//! after the other with the order alternating pair by pair; a run is `--reps`
//! rounds of 64 pairs, the first `WARM_REPS` rounds are warm-up and not
//! counted; `--runs` runs follow each other in one process and the command is
//! repeated in separate processes; p50 and p99 by nearest rank; the load
//! average is printed before and after each run. The verdict is printed with
//! its numbers and is "met" only if the ratio is within the ceiling in every
//! run; runs are never averaged away.
//!
//! ```text
//! cargo run --release -p kerno-units --example derived_bench2 -- --runs 4 --reps 12 --dir <scratch>
//! ```

// The loader of the frozen set is shared with the tests; the example uses part.
#[path = "../tests/harness/derived.rs"]
#[allow(dead_code)]
mod derived;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use kerno_core::contract::Options;
use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_testkit::derived_eval::{self, DIM, Scale, Story};
use kerno_units::assoc_rank::{VectorMetric, VectorQuery, VectorSet};
use kerno_units::*;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const P1_CEILING: f64 = 1.50;
const P2_CEILING: f64 = 1.10;
/// Pairs per round: as many as the 64 queries of Stage 7.
const PAIRS: usize = 64;
const REPS: usize = 40;
const WARM_REPS: usize = 4;
const K: usize = derived_eval::CUT;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config(corpus: &derived_eval::Corpus) -> UnitConfig {
    let mut c =
        UnitConfig::with_sources(Arc::new(FixedClock::new(derived::T0)), Arc::new(derived::PermutedIds::new(corpus)));
    c.engine = Options::default();
    c
}

fn open(dir: &std::path::Path, corpus: &derived_eval::Corpus) -> Res<DerivedDb<KernoDb>> {
    let db = UnitDb::<KernoDb>::open(StdVfs::shared(), dir, &config(corpus))?;
    Ok(DerivedDb::open_persisted(db)?.0)
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

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|_| "n/a".to_owned())
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, u64) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed().as_nanos() as u64)
}

/// One pair of the P1 measurement. `with_descriptor` writes the unit and its
/// persisted vector in one change set; otherwise only the unit.
fn write_source(dd: &DerivedDb<KernoDb>, vector: &[f32], n: usize, with_descriptor: bool) -> Res<()> {
    let mut cs = dd.begin_changeset(ChangeMeta::default())?;
    let new = NewUnit {
        unit_type: "fact".to_owned(),
        schema_version: 1,
        metadata: Vec::new(),
        payload: format!("bench source {n}").into_bytes(),
    };
    let id = cs.create(new)?;
    if with_descriptor {
        let head = cs.get_unit(id)?.ok_or("a created unit is not readable")?.head;
        cs.put_vector(&derived::spec(RevisionId { unit: id, rev: head }, vector.to_vec()))?;
    }
    cs.commit()?;
    Ok(())
}

struct Summary {
    run: usize,
    p50_a: f64,
    p50_b: f64,
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

    let story = Story::generate(Scale::Performance, kerno_testkit::derived_eval2::SEED);
    let dir = TempDir(base.join(format!("kerno-derived-bench-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0)?;

    let t = Instant::now();
    let dd = open(&dir.0, &story.corpus)?;
    let loaded = derived::load(&dd, &story.corpus, &story.vectors, true)?;
    dd.close()?;
    let load_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let dd = open(&dir.0, &story.corpus)?;
    let reopen_ms = t.elapsed().as_secs_f64() * 1e3;
    let n_units = story.corpus.units.len();

    // The persisted side: what the reopened database serves, read once.
    let t = Instant::now();
    let mut persisted = VectorSet::new(DIM, VectorMetric::Cosine)?;
    for (i, id) in loaded.derived.iter().enumerate() {
        let id = id.ok_or("a unit has no persisted vector")?;
        persisted.insert(loaded.units[i], dd.vector(&id).ok_or("a persisted vector is not served")?)?;
    }
    let read_ms = t.elapsed().as_secs_f64() * 1e3;
    // The in-memory side: the generator's vectors handed over by the caller.
    let mut memory = VectorSet::new(DIM, VectorMetric::Cosine)?;
    for (i, v) in story.vectors.iter().enumerate() {
        memory.insert(loaded.units[i], derived::f32_vector(v))?;
    }
    let queries: Vec<VectorQuery> = story
        .queries
        .queries
        .iter()
        .filter_map(|q| q.vector.as_ref())
        .map(|v| VectorQuery::new(VectorMetric::Cosine, derived::f32_vector(v)))
        .collect::<Result<_, _>>()?;
    if queries.is_empty() {
        return Err("the performance set has no vector query".into());
    }
    for q in &queries {
        if persisted.search(q, K)? != memory.search(q, K)? {
            return Err("the persisted and in-memory sets answer differently".into());
        }
    }
    println!(
        "# load: {} units, {} relations, {} change sets, {} vector queries; load time {load_s:.1} s; reopen {reopen_ms:.1} ms; \
         one-time read of {n_units} served vectors into a set {read_ms:.1} ms",
        n_units,
        story.corpus.relations.len(),
        story.corpus.changes.len(),
        queries.len()
    );

    println!("measure,run,arm,pairs,p50_ms,p99_ms,loadavg_before,loadavg_after");
    let (mut p1, mut p2): (Vec<Summary>, Vec<Summary>) = (Vec::new(), Vec::new());
    let mut written = 0usize;
    for run in 1..=runs {
        // P2: persisted (a) against in-memory (b), same query, same session.
        let before = loadavg();
        let (mut sa, mut sb) = (Vec::new(), Vec::new());
        for rep in 0..reps {
            for i in 0..PAIRS {
                let q = &queries[i % queries.len()];
                let (ta, tb);
                if (rep + i) % 2 == 0 {
                    ta = timed(|| persisted.search(q, K)).1;
                    tb = timed(|| memory.search(q, K)).1;
                } else {
                    tb = timed(|| memory.search(q, K)).1;
                    ta = timed(|| persisted.search(q, K)).1;
                }
                if rep >= WARM_REPS {
                    sa.push(ta);
                    sb.push(tb);
                }
            }
        }
        let after = loadavg();
        let pairs = sa.len();
        let (a50, a99) = percentiles(&mut sa);
        let (b50, b99) = percentiles(&mut sb);
        println!("P2,{run},persisted,{pairs},{a50:.4},{a99:.4},{before},{after}");
        println!("P2,{run},memory,{pairs},{b50:.4},{b99:.4},{before},{after}");
        println!("# run {run} P2: persisted/memory p50 = {:.3}, p99 = {:.3}", a50 / b50, a99 / b99);
        p2.push(Summary { run, p50_a: a50, p50_b: b50 });

        // P1: with descriptor (a) against without (b), alternating, same database.
        let before = loadavg();
        let (mut sa, mut sb) = (Vec::new(), Vec::new());
        for rep in 0..reps {
            for i in 0..PAIRS {
                let v = derived::f32_vector(&story.vectors[(written + i) % n_units]);
                let (ta, tb);
                if (rep + i) % 2 == 0 {
                    ta = timed(|| write_source(&dd, &v, written, true)).1;
                    tb = timed(|| write_source(&dd, &v, written + 1, false)).1;
                } else {
                    tb = timed(|| write_source(&dd, &v, written, false)).1;
                    ta = timed(|| write_source(&dd, &v, written + 1, true)).1;
                }
                written += 2;
                if rep >= WARM_REPS {
                    sa.push(ta);
                    sb.push(tb);
                }
            }
        }
        let after = loadavg();
        let pairs = sa.len();
        let (a50, a99) = percentiles(&mut sa);
        let (b50, b99) = percentiles(&mut sb);
        println!("P1,{run},with_descriptor,{pairs},{a50:.4},{a99:.4},{before},{after}");
        println!("P1,{run},without_descriptor,{pairs},{b50:.4},{b99:.4},{before},{after}");
        println!("# run {run} P1: with/without p50 = {:.3}, p99 = {:.3}", a50 / b50, a99 / b99);
        p1.push(Summary { run, p50_a: a50, p50_b: b50 });
    }

    for (code, what, ceiling, list) in
        [("P1", "with / without descriptor", P1_CEILING, &p1), ("P2", "persisted / in memory", P2_CEILING, &p2)]
    {
        println!("# {code} (acta §6.3): p50 {what} <= {ceiling:.2} in every run");
        for s in list {
            let ratio = s.p50_a / s.p50_b;
            println!(
                "#   run {}: {:.4} ms / {:.4} ms = {ratio:.3} -> {}",
                s.run,
                s.p50_a,
                s.p50_b,
                if ratio <= ceiling { "within" } else { "above" }
            );
        }
        let above = list.iter().filter(|s| s.p50_a / s.p50_b > ceiling).count();
        println!(
            "# {code}: {above} of {} runs above the ceiling -> {}",
            list.len(),
            if above == 0 { "met" } else { "not met" }
        );
    }
    dd.close()?;
    Ok(())
}
