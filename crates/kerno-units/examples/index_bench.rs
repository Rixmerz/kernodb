//! `index_bench`: workload W1 of `docs/architecture/etapa4-workloads.md`
//! (E4-04). Measures a selective `UnitFilter` query on `KernoDb` over the
//! real file system through three paths on the same snapshot:
//!
//! * `reference` — `reference_filter` (opens every head);
//! * `primary` — `IDX_PRIMARY` with the index `Ready` (E4-03);
//! * `secondary` — `query` with the index `Ready`, which answers a type
//!   filter from `IDX_TYPE` (E4-04, built because `primary` missed the
//!   threshold);
//! * `ideal` — `get_unit` of exactly the matching ids, the lower bound of
//!   any secondary index (which would also scan its own range).
//!
//! * `legacy` — what `query` did before the planner (E5-02): the index status
//!   and the automatic index path, with no cost estimate;
//!
//! E5-02 compares `secondary` (the planner) with `legacy` in rounds that
//! alternate the two and prints the overhead against the 10 % threshold of
//! the Stage 5 acta (§8, decision 5).
//!
//! E5-03 measures the cost of recording an observation: `secondary` with the
//! observer at `off` against `aggregate`, `fingerprint` and `diagnostic`, in
//! rounds that alternate the levels. The cost is the added p50; the threshold
//! of the Stage 5 acta (§8, decision 5) is 5 % of the p99 of the unobserved
//! query.
//!
//! The paths must return the same units; the run stops if they do not. The
//! threshold of the workload (§3) is evaluated and printed at the end.
//!
//! E6-04 measures the result cache (`--cache`, see [`cache_section`]): the
//! same stream of reads is run with the cache off and on, in rounds that
//! alternate the two, over W1 (80 % reads that repeat, 20 % interleaved
//! writes) and over a stream with no repeated read, the worst case. The
//! thresholds are those of the Stage 6 acta (§6) and are not parameters.
//!
//! ```text
//! cargo run --release -p kerno-units --example index_bench            # full
//! cargo run --release -p kerno-units --example index_bench -- --quick # smoke
//! cargo run --release -p kerno-units --example index_bench -- --cache # E6-04
//! ```
//!
//! Options: `--quick`, `--cache`, `--dir <path>`, `--seed <u64>`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use kerno_core::contract::{Options, TestHooks};
use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_testkit::rng::Rng;
use kerno_units::{
    ChangeMeta, CounterIds, FixedClock, IndexPath, IndexState, NewUnit, ObservationLevel, PlanPath, QuerySource,
    Timestamp, Unit, UnitConfig, UnitDb, UnitFilter, UnitId, UnitSnapshot, UnitUpdate,
};

type Res<T> = Result<T, Box<dyn std::error::Error>>;
type Units = UnitDb<KernoDb>;
/// One query path: the units matching the filter (`ids` is the expected
/// answer, used only by the ideal path).
type QueryPath = fn(&UnitSnapshot<KernoDb>, &UnitFilter, &[UnitId]) -> Res<Vec<Unit>>;

const TYPES: u64 = 100;
const SCHEMAS: u64 = 3;
const PAYLOAD: usize = 256;
const PER_CS: usize = 500;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(
        Arc::new(FixedClock::stepping(Timestamp(1_700_000_000_000_000), 1)),
        Arc::new(CounterIds::new(1)),
    );
    c.engine = Options::default();
    c
}

/// An active unit: `(type, schema, id)`.
type Entry = (u64, u64, UnitId);

/// `n` active units plus 10 % archived ones; returns every active one.
fn load(db: &Units, n: usize, seed: u64) -> Res<Vec<Entry>> {
    let mut rng = Rng::new(seed);
    let total = n + n / 10;
    let mut active = Vec::with_capacity(n);
    let mut archived = Vec::new();
    let mut made = 0;
    while made < total {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        for _ in 0..PER_CS.min(total - made) {
            let (t, s) = (rng.below(TYPES), 1 + rng.below(SCHEMAS));
            let payload = (0..PAYLOAD).map(|_| rng.below(256) as u8).collect();
            let id = cs.create(NewUnit {
                unit_type: format!("t{t:03}"),
                schema_version: s as u32,
                metadata: Vec::new(),
                payload,
            })?;
            if made < n {
                active.push((t, s, id));
            } else {
                archived.push(id);
            }
            made += 1;
        }
        cs.commit()?;
    }
    for chunk in archived.chunks(PER_CS) {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        for id in chunk {
            cs.archive(*id, None)?;
        }
        cs.commit()?;
    }
    TestHooks::flush(db.engine())?;
    TestHooks::wait_idle(db.engine())?;
    Ok(active)
}

fn percentile(samples: &mut [u64], q: f64) -> f64 {
    samples.sort_unstable();
    let rank = ((q * samples.len() as f64).ceil() as usize).clamp(1, samples.len());
    samples[rank - 1] as f64 / 1e6
}

fn reference(s: &UnitSnapshot<KernoDb>, f: &UnitFilter, _: &[UnitId]) -> Res<Vec<Unit>> {
    Ok(s.reference_filter(f)?)
}

fn primary(s: &UnitSnapshot<KernoDb>, f: &UnitFilter, _: &[UnitId]) -> Res<Vec<Unit>> {
    Ok(s.indexed_scan_via(IndexPath::Primary, f)?)
}

fn secondary(s: &UnitSnapshot<KernoDb>, f: &UnitFilter, _: &[UnitId]) -> Res<Vec<Unit>> {
    let out = s.query(f)?;
    if !matches!(out.source, QuerySource::Index { .. }) {
        return Err("the primary index is not Ready".into());
    }
    Ok(out.units)
}

/// `query` as it was before the planner: the status, then the automatic path.
fn legacy(s: &UnitSnapshot<KernoDb>, f: &UnitFilter, _: &[UnitId]) -> Res<Vec<Unit>> {
    let status = s.index_status()?;
    if status.state != IndexState::Ready {
        return Err("the primary index is not Ready".into());
    }
    Ok(s.indexed_scan(f)?)
}

fn ideal(s: &UnitSnapshot<KernoDb>, _: &UnitFilter, ids: &[UnitId]) -> Res<Vec<Unit>> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(s.get_unit(*id)?.ok_or("missing unit")?);
    }
    Ok(out)
}

struct Query {
    filter: UnitFilter,
    ids: Vec<UnitId>,
}

fn queries(active: &[Entry], combined: bool, count: usize, seed: u64) -> Vec<Query> {
    let mut rng = Rng::new(seed);
    (0..count)
        .map(|_| {
            let t = rng.below(TYPES);
            let s = combined.then(|| 1 + rng.below(SCHEMAS));
            let mut ids: Vec<UnitId> =
                active.iter().filter(|(ut, us, _)| *ut == t && s.is_none_or(|s| s == *us)).map(|e| e.2).collect();
            ids.sort();
            let filter = UnitFilter { unit_type: Some(format!("t{t:03}")), schema_version: s.map(|s| s as u32) };
            Query { filter, ids }
        })
        .collect()
}

/// `(p50, p95, p99)` in ms after a 10 % warm-up; checks every answer.
fn measure(db: &Units, qs: &[Query], path: QueryPath) -> Res<(f64, f64, f64)> {
    let warm = (qs.len() / 10).max(1);
    let mut samples = Vec::with_capacity(qs.len());
    let snap = db.snapshot()?;
    for (i, q) in qs.iter().enumerate() {
        let t = Instant::now();
        let units = path(&snap, &q.filter, &q.ids)?;
        let ns = t.elapsed().as_nanos() as u64;
        if units.iter().map(|u| u.id).collect::<Vec<_>>() != q.ids {
            return Err("the paths disagree".into());
        }
        if i >= warm {
            samples.push(ns);
        }
    }
    Ok((percentile(&mut samples, 0.50), percentile(&mut samples, 0.95), percentile(&mut samples, 0.99)))
}

/// Rounds of the planner-versus-legacy comparison.
const ROUNDS: usize = 5;

/// `(p50, p99)` of each path per round, the two alternating; the overhead is
/// taken on the best round of each (least disturbed), in percent.
fn overhead(db: &Units, qs: &[Query]) -> Res<(f64, f64)> {
    let (mut planned, mut before) = (Vec::new(), Vec::new());
    for _ in 0..ROUNDS {
        before.push(measure(db, qs, legacy as QueryPath)?);
        planned.push(measure(db, qs, secondary as QueryPath)?);
    }
    let best = |r: &[(f64, f64, f64)], i: usize| r.iter().map(|m| [m.0, m.1, m.2][i]).fold(f64::INFINITY, f64::min);
    let pct = |i: usize| (best(&planned, i) / best(&before, i) - 1.0) * 100.0;
    for (r, (p, l)) in planned.iter().zip(&before).enumerate() {
        println!("# round {r}: planner p50/p99 = {:.3}/{:.3} ms, legacy = {:.3}/{:.3} ms", p.0, p.2, l.0, l.2);
    }
    Ok((pct(0), pct(2)))
}

const LEVELS: [ObservationLevel; 4] =
    [ObservationLevel::Off, ObservationLevel::Aggregate, ObservationLevel::Fingerprint, ObservationLevel::Diagnostic];

/// Added p50 of each observed level over `off`, and the p99 of `off`, both in
/// ms and on the best round of each level; the level goes back to `off`.
fn observation_cost(db: &Units, qs: &[Query]) -> Res<(f64, Vec<(ObservationLevel, f64)>)> {
    let mut rounds: Vec<Vec<(f64, f64, f64)>> = vec![Vec::new(); LEVELS.len()];
    for _ in 0..ROUNDS {
        for (i, level) in LEVELS.iter().enumerate() {
            db.observer().set_level(*level)?;
            rounds[i].push(measure(db, qs, secondary as QueryPath)?);
            db.observer().clear();
        }
    }
    db.observer().set_level(ObservationLevel::Off)?;
    let best = |i: usize, k: usize| rounds[i].iter().map(|m| [m.0, m.1, m.2][k]).fold(f64::INFINITY, f64::min);
    let added = (1..LEVELS.len()).map(|i| (LEVELS[i], best(i, 0) - best(0, 0))).collect();
    Ok((best(0, 2), added))
}

// ---- E6-04: the result cache against the cache off -------------------------

/// Thresholds of the Stage 6 acta (§6), fixed before measuring (§8, decision
/// 4). Percent changes of the cache on over the cache off.
const W1_P50_MUST_GAIN: f64 = 20.0;
const W1_P99_MAY_LOSE: f64 = 5.0;
const WORST_P99_MAY_LOSE: f64 = 5.0;

/// Rounds per comparison; each round runs the stream once with the cache off
/// and once on, the order flipping every round.
const CACHE_ROUNDS: usize = 5;

/// W1 as the acta (§8, decision 5) sizes it: 80 % reads, 20 % writes. One
/// write, then [`READS_PER_WRITE`] reads of one query: the first read after a
/// write cannot be served (every commit moves `commit_seq`), the other three
/// repeat it.
const READS_PER_WRITE: usize = 4;

/// One step of a stream: an optional write, then reads of `qs[i]`.
struct Block {
    write: bool,
    reads: Vec<usize>,
}

/// What a stream is run against and how it is shaped.
struct Load<'a> {
    label: &'static str,
    scale: usize,
    qs: &'a [Query],
    blocks: Vec<Block>,
    /// Rebuild the index after each write (untimed), so every read can use
    /// it, as the E6-00 baseline did. Without it the first write leaves the
    /// index `Stale` and every read is the reference scan.
    rebuild: bool,
    /// The unit each write updates: no unit is added, so the sets stay the same size.
    target: UnitId,
}

#[derive(Clone, Copy)]
struct Run {
    p50: f64,
    p95: f64,
    p99: f64,
    timed: usize,
    hits: usize,
}

/// One query per type, in type order: the cache's slot is one per shape, and
/// all of these share the shape `type = ?`.
fn by_type(active: &[Entry]) -> Vec<Query> {
    queries_of_types(active, (0..TYPES).collect())
}

fn queries_of_types(active: &[Entry], types: Vec<u64>) -> Vec<Query> {
    types
        .into_iter()
        .map(|t| {
            let mut ids: Vec<UnitId> = active.iter().filter(|(ut, _, _)| *ut == t).map(|e| e.2).collect();
            ids.sort();
            Query { filter: UnitFilter { unit_type: Some(format!("t{t:03}")), schema_version: None }, ids }
        })
        .collect()
}

/// Query `n` of a stream: 37 is coprime with the 100 types, so consecutive
/// queries differ and a type comes back only after all the others.
fn nth_query(n: usize) -> usize {
    n * 37 % TYPES as usize
}

fn write_one(db: &Units, target: UnitId, n: usize) -> Res<()> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    let update = UnitUpdate { payload: vec![(n % 251) as u8; PAYLOAD], schema_version: None, metadata: None };
    cs.update(target, update, None)?;
    cs.commit()?;
    Ok(())
}

/// Runs the stream once with the cache `on` or off and times every read after
/// a 10 % warm-up. Writes and index rebuilds are not timed.
fn run_stream(db: &Units, load: &Load, on: bool) -> Res<Run> {
    let cache = db.result_cache();
    cache.set_enabled(false);
    cache.clear();
    cache.set_enabled(on);
    let total: usize = load.blocks.iter().map(|b| b.reads.len()).sum();
    let warm = (total / 10).max(1);
    let (mut seen, mut hits) = (0, 0);
    let mut samples = Vec::with_capacity(total);
    for (n, block) in load.blocks.iter().enumerate() {
        if block.write {
            write_one(db, load.target, n)?;
            if load.rebuild {
                db.rebuild_index()?;
            }
        }
        let snap = db.snapshot()?;
        for &i in &block.reads {
            let q = &load.qs[i];
            let t = Instant::now();
            let out = snap.query(&q.filter)?;
            let ns = t.elapsed().as_nanos() as u64;
            if out.units.iter().map(|u| u.id).collect::<Vec<_>>() != q.ids {
                return Err("the cache and the reference disagree".into());
            }
            let served = out.path == PlanPath::ResultCache;
            if served && !on {
                return Err("the cache is off but an answer came from it".into());
            }
            if load.rebuild && out.path == PlanPath::UnitsReference {
                return Err("the index is not Ready: the baseline would be the reference scan".into());
            }
            if seen >= warm {
                samples.push(ns);
                hits += usize::from(served);
            }
            seen += 1;
        }
    }
    let stats = cache.stats();
    if !on && (stats.reads, stats.writes, stats.hits) != (0, 0, 0) {
        return Err("the cache is off but took part".into());
    }
    cache.set_enabled(false);
    let timed = samples.len();
    Ok(Run {
        p50: percentile(&mut samples, 0.50),
        p95: percentile(&mut samples, 0.95),
        p99: percentile(&mut samples, 0.99),
        timed,
        hits,
    })
}

fn pct(on: f64, off: f64) -> f64 {
    (on / off - 1.0) * 100.0
}

/// The p50 and p99 of the cache on over the cache off, in percent, taking
/// the best round of each (the method of E6-00) or the median round.
fn deltas(on: &[Run], off: &[Run], best: bool) -> (f64, f64) {
    let pick = |rs: &[Run], f: fn(&Run) -> f64| {
        let mut v: Vec<f64> = rs.iter().map(f).collect();
        v.sort_by(f64::total_cmp);
        if best { v[0] } else { v[v.len() / 2] }
    };
    (pct(pick(on, |r| r.p50), pick(off, |r| r.p50)), pct(pick(on, |r| r.p99), pick(off, |r| r.p99)))
}

fn verdict(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}

/// Runs `load` for [`CACHE_ROUNDS`] rounds, off and on alternating, prints
/// every round and the comparison with the thresholds. `w1` selects which
/// thresholds apply: those of W1 (p50 gain and p99 loss) or the worst case.
fn compare(db: &Units, load: &Load, w1: bool) -> Res<()> {
    let (mut off, mut on) = (Vec::new(), Vec::new());
    for round in 0..CACHE_ROUNDS {
        let order = if round % 2 == 0 { [false, true] } else { [true, false] };
        for mode in order {
            let r = run_stream(db, load, mode)?;
            println!(
                "cache,{},{},{},{round},{},{},{:.3},{:.3},{:.3}",
                load.scale,
                load.label,
                if mode { "on" } else { "off" },
                r.timed,
                r.hits,
                r.p50,
                r.p95,
                r.p99
            );
            if mode { &mut on } else { &mut off }.push(r);
        }
    }
    let rate =
        |rs: &[Run]| rs.iter().map(|r| r.hits).sum::<usize>() as f64 / rs.iter().map(|r| r.timed).sum::<usize>() as f64;
    let head = format!("# {} {}", load.scale, load.label);
    println!("{head}: cache-on reads served by the cache: {:.1} %", rate(&on) * 100.0);
    if !w1 && on.iter().any(|r| r.hits != 0) {
        return Err("the stream without repeats was served from the cache".into());
    }
    for (name, best) in [("best round", true), ("median round", false)] {
        let (p50, p99) = deltas(&on, &off, best);
        let (limit, text) = if w1 {
            (
                W1_P99_MAY_LOSE,
                format!("p50 {p50:+.1} % (needs <= -{W1_P50_MUST_GAIN:.0} %: {})", verdict(p50 <= -W1_P50_MUST_GAIN)),
            )
        } else {
            (WORST_P99_MAY_LOSE, format!("p50 {p50:+.1} % (not a threshold)"))
        };
        println!(
            "{head}: cache on vs off, {name}: {text}, p99 {p99:+.1} % (needs <= +{limit:.0} %: {})",
            verdict(p99 <= limit)
        );
    }
    Ok(())
}

/// E6-04. `scale` units; the stream without repeats runs at every scale, W1
/// at the first only (a write plus an index rebuild per block).
fn cache_section(base: &std::path::Path, seed: u64, quick: bool) -> Res<()> {
    // (units, reads without repeats, W1 blocks with the index kept Ready, W1 blocks with the index left stale)
    let scales: &[(usize, usize, usize, usize)] =
        if quick { &[(2_000, 200, 20, 10)] } else { &[(20_000, 4_000, 400, 150), (100_000, 1_500, 0, 0)] };
    println!("cache,scale,load,cache,round,timed_reads,served,p50_ms,p95_ms,p99_ms");
    for &(n, norepeat, ready, stale) in scales {
        let dir = TempDir(base.join(format!("kerno-cache-bench-{}-{n}", std::process::id())));
        let _ = std::fs::remove_dir_all(&dir.0);
        std::fs::create_dir_all(&dir.0)?;
        let db = Units::open(StdVfs::shared(), &dir.0, &config())?;
        let active = load(&db, n, seed)?;
        db.rebuild_index()?;
        let qs = by_type(&active);
        let target = active[0].2;
        let stream = |blocks: usize, writes: bool| -> Vec<Block> {
            (0..blocks)
                .map(|b| Block { write: writes, reads: vec![nth_query(b); if writes { READS_PER_WRITE } else { 1 }] })
                .collect()
        };
        let plan = [
            ("no-repeat", false, norepeat, false, false),
            ("w1-index-ready", true, ready, true, true),
            ("w1-index-stale", true, stale, false, true),
        ];
        for (label, w1, blocks, rebuild, writes) in plan {
            if blocks == 0 {
                continue;
            }
            let case = Load { label, scale: n, qs: &qs, blocks: stream(blocks, writes), rebuild, target };
            compare(&db, &case, w1)?;
        }
        db.close()?;
    }
    Ok(())
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let base = arg("--dir").map_or_else(std::env::temp_dir, PathBuf::from);
    let seed: u64 = arg("--seed").map_or(Ok(42), |s| s.parse())?;
    if args.iter().any(|a| a == "--cache") {
        return cache_section(&base, seed, quick);
    }
    let scales: &[(usize, usize)] = if quick { &[(2_000, 40)] } else { &[(20_000, 220), (100_000, 110)] };
    let mut justified_at_100k = false;
    println!("scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms");
    for &(n, count) in scales {
        let dir = TempDir(base.join(format!("kerno-idx-bench-{}-{n}", std::process::id())));
        let _ = std::fs::remove_dir_all(&dir.0);
        std::fs::create_dir_all(&dir.0)?;
        let db = Units::open(StdVfs::shared(), &dir.0, &config())?;
        let active = load(&db, n, seed)?;
        let t = Instant::now();
        db.rebuild_index()?;
        let build_ms = t.elapsed().as_secs_f64() * 1e3;
        for (name, combined) in [("type", false), ("type+schema", true)] {
            let qs = queries(&active, combined, count, seed ^ n as u64);
            let hits = qs.iter().map(|q| q.ids.len()).sum::<usize>() as f64 / qs.len() as f64;
            let mut r = Vec::new();
            for (pname, p) in [
                ("reference", reference as QueryPath),
                ("primary", primary),
                ("ideal", ideal),
                ("secondary", secondary),
            ] {
                let m = measure(&db, &qs, p)?;
                println!("{n},{name},{pname},{:.3},{:.3},{:.3},{hits:.0},{build_ms:.0}", m.0, m.1, m.2);
                r.push(m);
            }
            let (prim, best) = (r[1], r[2]);
            let justified = prim.0 > 2.0 * best.0 && prim.1 > 10.0;
            println!(
                "# {n} {name}: primary/ideal p50 = {:.2}x, primary p95 = {:.3} ms, secondary/ideal p50 = {:.2}x -> {}",
                prim.0 / best.0,
                prim.1,
                r[3].0 / best.0,
                if justified { "threshold exceeded" } else { "threshold not exceeded" }
            );
            if n == 100_000 {
                justified_at_100k |= justified;
            }
            if !combined {
                let (p50, p99) = overhead(&db, &qs)?;
                println!(
                    "# {n} {name}: planner overhead over the current path: p50 {p50:+.1} %, p99 {p99:+.1} % (threshold 10 %) -> {}",
                    if p50 <= 10.0 && p99 <= 10.0 { "within" } else { "exceeded" }
                );
            }
            if !combined {
                let (p99_off, added) = observation_cost(&db, &qs)?;
                for (level, ms) in added {
                    let pct = ms / p99_off * 100.0;
                    println!(
                        "# {n} {name}: observation at {level:?} adds p50 {ms:+.4} ms = {pct:+.2} % of the unobserved p99 {p99_off:.3} ms (threshold 5 %) -> {}",
                        if pct <= 5.0 { "within" } else { "exceeded" }
                    );
                }
            }
        }
        db.close()?;
    }
    if !quick {
        println!("# secondary index justified by W1: {justified_at_100k}");
    }
    Ok(())
}
