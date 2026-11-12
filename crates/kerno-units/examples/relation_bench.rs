//! `relation_bench`: the prototype comparison of ADR-0014 (E4-02), option 1
//! against option 2, both on `KernoDb` opened with the same options on the
//! real file system (`StdVfs`), in the same process.
//!
//! * Option 1: a relation is a unit of a reserved type (`rel`), its payload
//!   `from ‖ to ‖ provenance ‖ relation_type`, no adjacency index. Neighbours
//!   are found by filtering every relation (`reference_by_type`); a 2-hop
//!   query reads all relations once and filters the copy twice (its best case).
//! * Option 2: `create_relation` (record plus `REL_OUT` and `REL_IN` in the
//!   same change set), neighbours by `relations_of` (a range scan).
//!
//! Criterion 3 of the ADR: cost per relation (keys and bytes written, bytes
//! stored) and latency of 1- and 2-hop traversals, on two synthetic shapes:
//! `livespec` (symbols and specs: out-degree ~5, a few hubs that receive
//! half of the edges) and `memory` (agent memory: a chain plus links to
//! recent items, out-degree ~3, no hubs). The two options must return the
//! same neighbours (criterion 2); the run stops if they do not.
//!
//! ```text
//! cargo run --release -p kerno-units --example relation_bench            # full
//! cargo run --release -p kerno-units --example relation_bench -- --quick # smoke
//! ```
//!
//! Options: `--quick`, `--dir <path>`, `--seed <u64>`.

use std::collections::BTreeSet;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use kerno_core::contract::{Options, Snapshot};
use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_testkit::rng::Rng;
use kerno_units::{
    ChangeMeta, CounterIds, Direction, FixedClock, NewUnit, RelationProvenance, Timestamp, UnitConfig, UnitDb, UnitId,
    UnitSnapshot,
};

type Res<T> = Result<T, Box<dyn std::error::Error>>;
type Units = UnitDb<KernoDb>;
/// One hop-expansion strategy: the units reached from `from` in `hops` hops.
type Traversal = fn(&UnitSnapshot<KernoDb>, &[UnitId], usize) -> Res<BTreeSet<UnitId>>;

const REL_TYPE: &str = "links";
/// Unit type of an option 1 relation.
const REL_UNIT: &str = "rel";

struct Shape {
    name: &'static str,
    nodes: usize,
    out_degree: usize,
    hubs: bool,
    /// Samples taken per option for the 1-hop and 2-hop latencies.
    queries_opt2: usize,
    queries_opt1: usize,
}

fn shapes(quick: bool) -> Vec<Shape> {
    let (n, q2, q1) = if quick { (500, 200, 20) } else { (5_000, 2_000, 200) };
    vec![
        Shape { name: "livespec", nodes: n, out_degree: 5, hubs: true, queries_opt2: q2, queries_opt1: q1 },
        Shape { name: "memory", nodes: n, out_degree: 3, hubs: false, queries_opt2: q2, queries_opt1: q1 },
    ]
}

/// `(from, to)` as node indexes. Deterministic from the seed.
fn edges(shape: &Shape, seed: u64) -> Vec<(usize, usize)> {
    let mut rng = Rng::new(seed);
    let n = shape.nodes;
    let mut out = Vec::with_capacity(n * shape.out_degree);
    for from in 0..n {
        for k in 0..shape.out_degree {
            let to = if shape.hubs {
                if rng.chance(1, 2) { rng.below((n / 20).max(1) as u64) } else { rng.below(n as u64) }
            } else if k == 0 {
                from.saturating_sub(1) as u64
            } else {
                from.saturating_sub(1 + rng.below(50) as usize) as u64
            };
            out.push((from, to as usize));
        }
    }
    out
}

fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(
        Arc::new(FixedClock::stepping(Timestamp(1_700_000_000_000_000), 1)),
        Arc::new(CounterIds::new(1)),
    );
    c.engine = Options::default();
    c
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(base: &Path, name: &str) -> Res<TempDir> {
        let dir = base.join(format!("kerno-rel-bench-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stat(db: &Units, name: &str) -> u64 {
    db.storage_stats().into_iter().find(|(n, _)| n == name).map_or(0, |(_, v)| v)
}

fn stored(db: &Units) -> u64 {
    let level = |n: &str| {
        n.strip_prefix('l')
            .and_then(|r| r.strip_suffix("_bytes"))
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    };
    db.storage_stats().into_iter().filter(|(n, _)| level(n)).map(|(_, v)| v).sum()
}

fn settle(db: &Units) -> Res<()> {
    db.hook_flush()?;
    db.hook_wait_idle()?;
    Ok(())
}

/// Number of keys in the whole store at the latest commit.
fn key_count(db: &Units) -> Res<u64> {
    let snap = db.raw_snapshot()?;
    let mut n = 0u64;
    let mut last: Option<Vec<u8>> = None;
    loop {
        let lo = last.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
        let page = snap.scan(lo, Bound::Unbounded, 1024)?;
        n += page.len() as u64;
        if page.len() < 1024 {
            return Ok(n);
        }
        last = page.into_iter().last().map(|(k, _)| k);
    }
}

fn percentile(samples: &mut [u64], q: f64) -> f64 {
    samples.sort_unstable();
    let rank = ((q * samples.len() as f64).ceil() as usize).clamp(1, samples.len());
    samples[rank - 1] as f64 / 1e6
}

struct Cost {
    keys_per_rel: f64,
    wal_per_rel: f64,
    flush_per_rel: f64,
    compaction_per_rel: f64,
    stored_per_rel: f64,
    load_secs: f64,
}

struct Built {
    db: Units,
    nodes: Vec<UnitId>,
    cost: Cost,
    _dir: TempDir,
}

fn node_unit(i: usize) -> NewUnit {
    NewUnit {
        unit_type: "node".into(),
        schema_version: 1,
        metadata: Vec::new(),
        payload: (i as u64).to_be_bytes().to_vec(),
    }
}

fn rel_payload(from: UnitId, to: UnitId) -> Vec<u8> {
    // from ‖ to ‖ provenance (0 = declared) ‖ relation_type
    let mut p = Vec::with_capacity(33 + REL_TYPE.len());
    p.extend_from_slice(&from.0);
    p.extend_from_slice(&to.0);
    p.push(0);
    p.extend_from_slice(REL_TYPE.as_bytes());
    p
}

fn build(base: &Path, shape: &Shape, es: &[(usize, usize)], option: u8) -> Res<Built> {
    const PER_CS: usize = 500;
    let dir = TempDir::new(base, &format!("{}-opt{option}", shape.name))?;
    let db = Units::open(StdVfs::shared(), &dir.0, &config())?;
    let mut nodes = Vec::with_capacity(shape.nodes);
    for chunk in (0..shape.nodes).collect::<Vec<_>>().chunks(PER_CS) {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        for i in chunk {
            nodes.push(cs.create(node_unit(*i))?);
        }
        cs.commit()?;
    }
    settle(&db)?;
    let (keys0, wal0, flush0, comp0) = (
        key_count(&db)?,
        stat(&db, "wal_bytes"),
        stat(&db, "flush_bytes_written"),
        stat(&db, "compaction_bytes_written"),
    );
    let stored0 = stored(&db);
    let t = Instant::now();
    for chunk in es.chunks(PER_CS) {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        for (f, to) in chunk {
            if option == 2 {
                cs.create_relation(nodes[*f], nodes[*to], REL_TYPE, RelationProvenance::Declared)?;
            } else {
                cs.create(NewUnit {
                    unit_type: REL_UNIT.into(),
                    schema_version: 1,
                    metadata: Vec::new(),
                    payload: rel_payload(nodes[*f], nodes[*to]),
                })?;
            }
        }
        cs.commit()?;
    }
    let load_secs = t.elapsed().as_secs_f64();
    settle(&db)?;
    let n = es.len() as f64;
    let d = |a: u64, b: u64| (a as f64 - b as f64) / n;
    let cost = Cost {
        keys_per_rel: d(key_count(&db)?, keys0),
        wal_per_rel: d(stat(&db, "wal_bytes"), wal0),
        flush_per_rel: d(stat(&db, "flush_bytes_written"), flush0),
        compaction_per_rel: d(stat(&db, "compaction_bytes_written"), comp0),
        stored_per_rel: d(stored(&db), stored0),
        load_secs,
    };
    Ok(Built { db, nodes, cost, _dir: dir })
}

/// Option 1: every relation, read once.
fn opt1_all(snap: &UnitSnapshot<KernoDb>) -> Res<Vec<(UnitId, UnitId)>> {
    let units = snap.reference_by_type(REL_UNIT)?;
    Ok(units
        .iter()
        .map(|u| {
            let p = u.payload.as_deref().expect("payload");
            (UnitId(p[..16].try_into().expect("from")), UnitId(p[16..32].try_into().expect("to")))
        })
        .collect())
}

fn opt1_hop(snap: &UnitSnapshot<KernoDb>, from: &[UnitId], hops: usize) -> Res<BTreeSet<UnitId>> {
    let all = opt1_all(snap)?;
    let mut frontier: BTreeSet<UnitId> = from.iter().copied().collect();
    for _ in 0..hops {
        frontier = all.iter().filter(|(f, _)| frontier.contains(f)).map(|(_, t)| *t).collect();
    }
    Ok(frontier)
}

fn opt2_hop(snap: &UnitSnapshot<KernoDb>, from: &[UnitId], hops: usize) -> Res<BTreeSet<UnitId>> {
    let mut frontier: BTreeSet<UnitId> = from.iter().copied().collect();
    for _ in 0..hops {
        let mut next = BTreeSet::new();
        for u in &frontier {
            next.extend(snap.relations_of(*u, Direction::Out, None)?.into_iter().map(|r| r.to));
        }
        frontier = next;
    }
    Ok(frontier)
}

fn latencies(db: &Units, nodes: &[UnitId], hops: usize, queries: usize, seed: u64, f: Traversal) -> Res<Vec<u64>> {
    let mut rng = Rng::new(seed);
    let warm = (queries / 10).max(1);
    let mut out = Vec::with_capacity(queries);
    for i in 0..warm + queries {
        let id = nodes[rng.below(nodes.len() as u64) as usize];
        let t = Instant::now();
        let snap = db.snapshot()?;
        let r = f(&snap, &[id], hops)?;
        std::hint::black_box(r);
        if i >= warm {
            out.push(t.elapsed().as_nanos() as u64);
        }
    }
    Ok(out)
}

fn main() -> Res<()> {
    let (mut quick, mut dir, mut seed) = (false, std::env::temp_dir(), 0xE402u64);
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--quick" => quick = true,
            "--dir" => dir = PathBuf::from(it.next().ok_or("--dir needs a path")?),
            "--seed" => seed = it.next().ok_or("--seed needs a value")?.parse()?,
            other => return Err(format!("unknown argument {other:?}").into()),
        }
    }
    println!(
        "# relation_bench mode={} seed={seed:#x} dir={} engine=KernoDb vfs=StdVfs",
        if quick { "quick" } else { "full" },
        dir.display()
    );
    for shape in shapes(quick) {
        let es = edges(&shape, seed);
        println!("## {}: {} nodes, {} relations", shape.name, shape.nodes, es.len());
        let o1 = build(&dir, &shape, &es, 1)?;
        let o2 = build(&dir, &shape, &es, 2)?;
        for (opt, b) in [(1, &o1), (2, &o2)] {
            let c = &b.cost;
            println!(
                "raw {} opt{opt} cost: keys_per_rel={:.2} wal_B_per_rel={:.1} flush_B_per_rel={:.1} compaction_B_per_rel={:.1} stored_B_per_rel={:.1} load_s={:.2}",
                shape.name,
                c.keys_per_rel,
                c.wal_per_rel,
                c.flush_per_rel,
                c.compaction_per_rel,
                c.stored_per_rel,
                c.load_secs
            );
        }
        // Criterion 2: the same neighbours, 1 and 2 hops, for a sample of nodes.
        let mut rng = Rng::new(seed ^ 0xEC);
        let (s1, s2) = (o1.db.snapshot()?, o2.db.snapshot()?);
        for _ in 0..20 {
            let i = rng.below(shape.nodes as u64) as usize;
            for hops in [1, 2] {
                let a: BTreeSet<usize> = opt1_hop(&s1, &[o1.nodes[i]], hops)?
                    .into_iter()
                    .map(|u| o1.nodes.iter().position(|n| *n == u).expect("node"))
                    .collect();
                let b: BTreeSet<usize> = opt2_hop(&s2, &[o2.nodes[i]], hops)?
                    .into_iter()
                    .map(|u| o2.nodes.iter().position(|n| *n == u).expect("node"))
                    .collect();
                assert_eq!(a, b, "{}: node {i}, {hops} hop(s): options disagree", shape.name);
            }
        }
        println!("equivalence {}: opt1 == opt2 for 20 nodes x {{1, 2}} hops", shape.name);
        drop((s1, s2));
        for hops in [1, 2] {
            let mut l1 = latencies(&o1.db, &o1.nodes, hops, shape.queries_opt1, seed ^ 0x11, opt1_hop)?;
            let mut l2 = latencies(&o2.db, &o2.nodes, hops, shape.queries_opt2, seed ^ 0x22, opt2_hop)?;
            for (opt, l) in [(1, &mut l1), (2, &mut l2)] {
                println!(
                    "raw {} opt{opt} {hops}-hop: n={} p50_ms={:.4} p99_ms={:.4}",
                    shape.name,
                    l.len(),
                    percentile(l, 0.50),
                    percentile(l, 0.99)
                );
            }
        }
    }
    Ok(())
}
