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
//! The three must return the same units; the run stops if they do not. The
//! threshold of the workload (§3) is evaluated and printed at the end.
//!
//! ```text
//! cargo run --release -p kerno-units --example index_bench            # full
//! cargo run --release -p kerno-units --example index_bench -- --quick # smoke
//! ```
//!
//! Options: `--quick`, `--dir <path>`, `--seed <u64>`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use kerno_core::contract::{Options, TestHooks};
use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_testkit::rng::Rng;
use kerno_units::{
    ChangeMeta, CounterIds, FixedClock, IndexPath, NewUnit, QuerySource, Timestamp, Unit, UnitConfig, UnitDb,
    UnitFilter, UnitId, UnitSnapshot,
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

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let base = arg("--dir").map_or_else(std::env::temp_dir, PathBuf::from);
    let seed: u64 = arg("--seed").map_or(Ok(42), |s| s.parse())?;
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
                if justified { "threshold missed" } else { "threshold met" }
            );
            if n == 100_000 {
                justified_at_100k |= justified;
            }
        }
        db.close()?;
    }
    if !quick {
        println!("# secondary index justified by W1: {justified_at_100k}");
    }
    Ok(())
}
