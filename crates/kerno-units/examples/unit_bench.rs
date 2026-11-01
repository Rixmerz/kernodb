//! `unit_bench`: the `U` metrics of Stage 3 (etapa3-design.md §9.4, WP E3-09,
//! thresholds of decision 12). Every number is measured on `UnitDb<KernoDb>`
//! and on its internal reference, plain keys in `KernoDb`, opened with the
//! same options on the real file system (`StdVfs`), in the same process,
//! with the same warmup; the latency runs alternate reference and layer
//! round by round so both see the same machine load.
//!
//! ```text
//! cargo run --release -p kerno-units --example unit_bench            # full run
//! cargo run --release -p kerno-units --example unit_bench -- --quick # CI smoke
//! ```
//!
//! Options: `--quick`, `--dir <path>` (default: the system temp dir),
//! `--seed <u64>` (default 0xE309), `--only U1,U3,...`.
//!
//! * U1: p99 of a `Sync` change set with one 1 KiB `update` against the p99
//!   of a `Sync` commit of one 1 KiB put. Threshold <= 1.5x.
//! * U2: p99 of `history(id, latest(100))` (metadata only) against the p99 of
//!   a snapshot + `scan` of 100 keys of 100 B. Threshold <= 1.5x.
//! * U3: p99 of `unit_as_of` on units with 1 000 revisions against the p99 of
//!   a snapshot + `get` of a 1 KiB key. Reported; flagged above 5x.
//! * U4: WAL, flush and compaction bytes per `update` of 1 KiB and 64 KiB
//!   against a plain put of the same value (also the WAL cost of an update
//!   deferred from E3-08). Reported.
//! * U5: stored bytes per revision after N updates, after a full
//!   compaction, against N distinct plain keys of the same size. Reported.
//! * U6: writer p99 and throughput (plus a reader's p99) before and during
//!   `run_retention`, with the same concurrent load, and the retention
//!   throughput. Pass: p99 during <= the worse of two "before" windows and
//!   throughput during >= the lower of them.
//!
//! The process exits with an error only if an operation fails; a threshold
//! that is not met is printed as FAIL, it does not stop the run.

use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Snapshot, TestHooks};
use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_testkit::rng::Rng;
use kerno_units::{
    AsOf, ChangeMeta, CommitSeq, CounterIds, FixedClock, HistoryQuery, NewUnit, RetentionBudget, RetentionPolicy,
    RetentionRule, Timestamp, UnitAt, UnitConfig, UnitDb, UnitId, UnitUpdate,
};

type Res<T> = Result<T, Box<dyn std::error::Error>>;
type Units = UnitDb<KernoDb>;

// ------------------------------------------------------------ setup

struct Args {
    quick: bool,
    dir: PathBuf,
    seed: u64,
    only: Option<Vec<String>>,
}

fn parse_args() -> Res<Args> {
    let mut a = Args { quick: false, dir: std::env::temp_dir(), seed: 0xE309, only: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--quick" => a.quick = true,
            "--dir" => a.dir = PathBuf::from(it.next().ok_or("--dir needs a path")?),
            "--seed" => a.seed = it.next().ok_or("--seed needs a value")?.parse()?,
            "--only" => {
                a.only = Some(it.next().ok_or("--only needs a list")?.split(',').map(str::to_uppercase).collect());
            }
            other => return Err(format!("unknown argument {other:?}").into()),
        }
    }
    Ok(a)
}

/// The engine options of every database of the run: the defaults
/// (`Sync`, 64 MiB cache, 8 MiB write buffer).
fn options() -> Options {
    Options::default()
}

fn unit_config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(
        Arc::new(FixedClock::stepping(Timestamp(1_700_000_000_000_000), 1)),
        Arc::new(CounterIds::new(1)),
    );
    c.engine = options();
    c
}

/// A fresh directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(base: &Path, name: &str) -> Res<TempDir> {
        let dir = base.join(format!("kerno-unit-bench-{}-{name}", std::process::id()));
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

fn open_units(dir: &Path) -> Res<Units> {
    Ok(Units::open(StdVfs::shared(), dir, &unit_config())?)
}

fn open_plain(dir: &Path) -> Res<KernoDb> {
    Ok(<KernoDb as Engine>::open(StdVfs::shared(), dir, &options())?)
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    Engine::stats(db).into_iter().find(|(n, _)| n == name).map_or(0, |(_, v)| v)
}

/// Bytes the engine wrote: WAL, flushes, compactions.
#[derive(Clone, Copy, Debug, Default)]
struct Written {
    wal: u64,
    flush: u64,
    compaction: u64,
}

fn written(db: &KernoDb) -> Written {
    Written {
        wal: stat(db, "wal_bytes"),
        flush: stat(db, "flush_bytes_written"),
        compaction: stat(db, "compaction_bytes_written"),
    }
}

/// Live table bytes (sum of `l<N>_bytes`).
fn stored(db: &KernoDb) -> u64 {
    let level = |n: &str| {
        n.strip_prefix('l')
            .and_then(|r| r.strip_suffix("_bytes"))
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    };
    Engine::stats(db).into_iter().filter(|(n, _)| level(n)).map(|(_, v)| v).sum()
}

fn plain_key(i: u64) -> Vec<u8> {
    // 16 bytes like a unit id, under a tag the layer does not use.
    let mut k = vec![0x80u8];
    k.extend_from_slice(&[0; 7]);
    k.extend_from_slice(&i.to_be_bytes());
    k
}

fn new_unit(payload: Vec<u8>) -> NewUnit {
    NewUnit { unit_type: "bench".into(), schema_version: 1, metadata: Vec::new(), payload }
}

fn upd(payload: Vec<u8>) -> UnitUpdate {
    UnitUpdate { payload, schema_version: None, metadata: None }
}

/// Creates `n` units with `len`-byte payloads, `per_cs` per change set.
fn create_units(db: &Units, rng: &mut Rng, n: usize, len: usize, per_cs: usize) -> Res<Vec<UnitId>> {
    let mut ids = Vec::with_capacity(n);
    while ids.len() < n {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        for _ in 0..per_cs.min(n - ids.len()) {
            ids.push(cs.create(new_unit(rng.bytes(len)))?);
        }
        cs.commit()?;
    }
    Ok(ids)
}

/// Adds `revs` revisions to every unit of `ids`: one change set per round
/// and per chunk of `per_cs` units.
fn add_revisions(db: &Units, rng: &mut Rng, ids: &[UnitId], revs: usize, len: usize, per_cs: usize) -> Res<()> {
    for _ in 0..revs {
        for chunk in ids.chunks(per_cs) {
            let mut cs = db.begin_changeset(ChangeMeta::default())?;
            for id in chunk {
                cs.update(*id, upd(rng.bytes(len)), None)?;
            }
            cs.commit()?;
        }
    }
    Ok(())
}

fn update_one(db: &Units, id: UnitId, payload: Vec<u8>) -> Res<()> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    cs.update(id, upd(payload), None)?;
    cs.commit()?;
    Ok(())
}

fn put_one(db: &KernoDb, key: Vec<u8>, value: Vec<u8>) -> Res<()> {
    let mut b = WriteBatch::new();
    b.put(key, value);
    Engine::commit(db, &b)?;
    Ok(())
}

/// Settles both engines (memtables flushed, background work done) so that
/// read measurements start from the same shape.
fn settle(db: &KernoDb) -> Res<()> {
    TestHooks::flush(db)?;
    TestHooks::wait_idle(db)?;
    Ok(())
}

// ------------------------------------------------------------ statistics

#[derive(Clone, Copy, Debug)]
struct Lat {
    n: usize,
    p50: f64,
    p99: f64,
    max: f64,
    mean: f64,
}

/// Nearest-rank percentiles of nanosecond samples, in ms.
fn lat(samples: &[u64]) -> Lat {
    let mut s = samples.to_vec();
    s.sort_unstable();
    let pick = |q: f64| {
        let rank = ((q * s.len() as f64).ceil() as usize).clamp(1, s.len());
        s[rank - 1] as f64 / 1e6
    };
    Lat {
        n: s.len(),
        p50: pick(0.50),
        p99: pick(0.99),
        max: *s.last().unwrap_or(&0) as f64 / 1e6,
        mean: s.iter().sum::<u64>() as f64 / s.len().max(1) as f64 / 1e6,
    }
}

fn show(tag: &str, l: &Lat) {
    println!("raw {tag}: n={} p50_ms={:.4} p99_ms={:.4} max_ms={:.4} mean_ms={:.4}", l.n, l.p50, l.p99, l.max, l.mean);
}

fn timed(f: impl FnOnce() -> Res<()>) -> Res<u64> {
    let t = Instant::now();
    f()?;
    Ok(t.elapsed().as_nanos() as u64)
}

struct Verdict {
    id: &'static str,
    text: String,
    pass: Option<bool>,
}

fn verdict(out: &mut Vec<Verdict>, id: &'static str, text: String, pass: Option<bool>) {
    let mark = match pass {
        Some(true) => "PASS",
        Some(false) => "FAIL",
        None => "INFO",
    };
    println!("verdict {id}: {mark} {text}");
    out.push(Verdict { id, text, pass });
}

// ------------------------------------------------------------ sizes

struct Sizes {
    rounds: usize,
    u1_units: usize,
    u1_warmup: usize,
    u1_ops: usize,
    u2_units: usize,
    u2_queries: usize,
    u3_units: usize,
    u3_revs: usize,
    u3_queries: usize,
    u4_ops_1k: usize,
    u4_ops_64k: usize,
    u5_units: usize,
    u5_revs: usize,
    u6_units: usize,
    u6_revs: usize,
    u6_hot: usize,
    u6_writers: usize,
    u6_window: Duration,
}

fn sizes(quick: bool) -> Sizes {
    if quick {
        Sizes {
            rounds: 1,
            u1_units: 100,
            u1_warmup: 20,
            u1_ops: 100,
            u2_units: 20,
            u2_queries: 200,
            u3_units: 1,
            u3_revs: 1_000,
            u3_queries: 200,
            u4_ops_1k: 100,
            u4_ops_64k: 30,
            u5_units: 20,
            u5_revs: 20,
            u6_units: 500,
            u6_revs: 5,
            u6_hot: 10,
            u6_writers: 2,
            u6_window: Duration::from_millis(300),
        }
    } else {
        Sizes {
            rounds: 5,
            u1_units: 1_000,
            u1_warmup: 200,
            u1_ops: 1_000,
            u2_units: 100,
            u2_queries: 5_000,
            u3_units: 10,
            u3_revs: 1_000,
            u3_queries: 5_000,
            u4_ops_1k: 2_000,
            u4_ops_64k: 500,
            u5_units: 100,
            u5_revs: 100,
            u6_units: 10_000,
            u6_revs: 20,
            u6_hot: 100,
            u6_writers: 2,
            u6_window: Duration::from_secs(3),
        }
    }
}

// ------------------------------------------------------------ U1

fn u1(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    const LEN: usize = 1024;
    println!("## U1: Sync commit of one 1 KiB update vs one 1 KiB put");
    let (du, dp) = (TempDir::new(&a.dir, "u1-units")?, TempDir::new(&a.dir, "u1-plain")?);
    let mut rng = Rng::new(a.seed ^ 0x01);
    let units = open_units(&du.0)?;
    let plain = open_plain(&dp.0)?;
    let ids = create_units(&units, &mut rng, z.u1_units, LEN, 1_000)?;
    for chunk in (0..z.u1_units as u64).collect::<Vec<_>>().chunks(1_000) {
        let mut b = WriteBatch::new();
        for i in chunk {
            b.put(plain_key(*i), rng.bytes(LEN));
        }
        Engine::commit(&plain, &b)?;
    }
    let (mut su, mut sp) = (Vec::new(), Vec::new());
    for round in 0..z.rounds {
        // Alternate which side goes first in each round.
        for side in [round % 2, 1 - round % 2] {
            let mut lats = Vec::with_capacity(z.u1_ops);
            for i in 0..z.u1_warmup + z.u1_ops {
                let pick = rng.below(z.u1_units as u64);
                let payload = rng.bytes(LEN);
                let ns = if side == 0 {
                    timed(|| update_one(&units, ids[pick as usize], payload))?
                } else {
                    timed(|| put_one(&plain, plain_key(pick), payload))?
                };
                if i >= z.u1_warmup {
                    lats.push(ns);
                }
            }
            let name = if side == 0 { "update" } else { "put" };
            show(&format!("U1 round {round} {name}"), &lat(&lats));
            if side == 0 { su.extend(lats) } else { sp.extend(lats) }
        }
    }
    let (lu, lp) = (lat(&su), lat(&sp));
    show("U1 update (all rounds)", &lu);
    show("U1 put (all rounds)", &lp);
    let ratio = lu.p99 / lp.p99;
    verdict(
        out,
        "U1",
        format!("p99 update {:.3} ms / put {:.3} ms = {ratio:.2}x (threshold <= 1.5x)", lu.p99, lp.p99),
        Some(ratio <= 1.5),
    );
    units.close()?;
    plain.close()?;
    Ok(())
}

// ------------------------------------------------------------ U2

fn u2(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    const LEN: usize = 100;
    const REVS: usize = 100;
    println!("## U2: history(id, latest(100)) metadata only vs scan of 100 keys of 100 B");
    let (du, dp) = (TempDir::new(&a.dir, "u2-units")?, TempDir::new(&a.dir, "u2-plain")?);
    let mut rng = Rng::new(a.seed ^ 0x02);
    let units = open_units(&du.0)?;
    let plain = open_plain(&dp.0)?;
    let ids = create_units(&units, &mut rng, z.u2_units, LEN, 1_000)?;
    add_revisions(&units, &mut rng, &ids, REVS - 1, LEN, 1_000)?;
    let keys = (z.u2_units * REVS) as u64;
    for chunk in (0..keys).collect::<Vec<_>>().chunks(1_000) {
        let mut b = WriteBatch::new();
        for i in chunk {
            b.put(plain_key(*i), rng.bytes(LEN));
        }
        Engine::commit(&plain, &b)?;
    }
    settle(units.engine())?;
    settle(&plain)?;
    let (mut su, mut sp) = (Vec::new(), Vec::new());
    let warm = z.u2_queries / 10;
    for round in 0..z.rounds {
        for side in [round % 2, 1 - round % 2] {
            for i in 0..warm + z.u2_queries {
                let ns = if side == 0 {
                    let id = ids[rng.below(ids.len() as u64) as usize];
                    timed(|| {
                        let h = units.history(id, HistoryQuery::latest(REVS))?;
                        if h.len() != REVS {
                            return Err(format!("history returned {} revisions", h.len()).into());
                        }
                        Ok(())
                    })?
                } else {
                    let start = plain_key(rng.below(keys - REVS as u64 + 1));
                    timed(|| {
                        let snap = plain.snapshot()?;
                        let page = snap.scan(Bound::Included(&start), Bound::Unbounded, REVS)?;
                        if page.len() != REVS {
                            return Err(format!("scan returned {} keys", page.len()).into());
                        }
                        Ok(())
                    })?
                };
                if i >= warm {
                    if side == 0 { su.push(ns) } else { sp.push(ns) }
                }
            }
        }
    }
    let (lu, lp) = (lat(&su), lat(&sp));
    show("U2 history", &lu);
    show("U2 scan", &lp);
    let ratio = lu.p99 / lp.p99;
    verdict(
        out,
        "U2",
        format!("p99 history {:.4} ms / scan {:.4} ms = {ratio:.2}x (threshold <= 1.5x)", lu.p99, lp.p99),
        Some(ratio <= 1.5),
    );
    units.close()?;
    plain.close()?;
    Ok(())
}

// ------------------------------------------------------------ U3

fn u3(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    const LEN: usize = 1024;
    println!("## U3: unit_as_of with {} revisions vs get of a 1 KiB key", z.u3_revs);
    let (du, dp) = (TempDir::new(&a.dir, "u3-units")?, TempDir::new(&a.dir, "u3-plain")?);
    let mut rng = Rng::new(a.seed ^ 0x03);
    let units = open_units(&du.0)?;
    let plain = open_plain(&dp.0)?;
    let first = units.engine().snapshot()?.seq() + 1;
    let ids = create_units(&units, &mut rng, z.u3_units, LEN, 1_000)?;
    add_revisions(&units, &mut rng, &ids, z.u3_revs - 1, LEN, 1_000)?;
    let last = units.engine().snapshot()?.seq();
    let keys = (z.u3_units * z.u3_revs) as u64;
    for chunk in (0..keys).collect::<Vec<_>>().chunks(1_000) {
        let mut b = WriteBatch::new();
        for i in chunk {
            b.put(plain_key(*i), rng.bytes(LEN));
        }
        Engine::commit(&plain, &b)?;
    }
    settle(units.engine())?;
    settle(&plain)?;
    let (mut su, mut sp, mut sh) = (Vec::new(), Vec::new(), Vec::new());
    let warm = z.u3_queries / 10;
    for round in 0..z.rounds {
        for side in [round % 2, 1 - round % 2] {
            for i in 0..warm + z.u3_queries {
                if side == 0 {
                    let id = ids[rng.below(ids.len() as u64) as usize];
                    let at = CommitSeq(rng.range(first, last));
                    let ns = timed(|| match units.unit_as_of(id, AsOf::Seq(at))? {
                        UnitAt::Present(_) => Ok(()),
                        other => Err(format!("unit_as_of returned {other:?}").into()),
                    })?;
                    let nh = timed(|| units.get_unit(id).map(drop).map_err(Into::into))?;
                    if i >= warm {
                        su.push(ns);
                        sh.push(nh);
                    }
                } else {
                    let k = plain_key(rng.below(keys));
                    let ns = timed(|| {
                        plain.snapshot()?.get(&k)?.ok_or("missing key")?;
                        Ok(())
                    })?;
                    if i >= warm {
                        sp.push(ns);
                    }
                }
            }
        }
    }
    let (lu, lp, lh) = (lat(&su), lat(&sp), lat(&sh));
    show("U3 unit_as_of", &lu);
    show("U3 get (plain)", &lp);
    show("U3 get_unit (head, context)", &lh);
    let ratio = lu.p99 / lp.p99;
    let flag = if ratio > 5.0 { " FLAG: above 5x a get (design §3.5: inverted index only if justified)" } else { "" };
    verdict(
        out,
        "U3",
        format!("p99 unit_as_of {:.4} ms / get {:.4} ms = {ratio:.2}x (report; flag > 5x){flag}", lu.p99, lp.p99),
        None,
    );
    units.close()?;
    plain.close()?;
    Ok(())
}

// ------------------------------------------------------------ U4

fn u4(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    println!("## U4: bytes written per update vs per plain put (WAL, flush, compaction)");
    for (len, ops) in [(1024usize, z.u4_ops_1k), (64 * 1024, z.u4_ops_64k)] {
        let units_n = 100usize;
        let (du, dp) = (TempDir::new(&a.dir, "u4-units")?, TempDir::new(&a.dir, "u4-plain")?);
        let mut rng = Rng::new(a.seed ^ 0x04 ^ len as u64);
        let units = open_units(&du.0)?;
        let plain = open_plain(&dp.0)?;
        let ids = create_units(&units, &mut rng, units_n, len, 100)?;
        for i in 0..units_n as u64 {
            put_one(&plain, plain_key(i), rng.bytes(len))?;
        }
        settle(units.engine())?;
        settle(&plain)?;
        let (wu0, wp0) = (written(units.engine()), written(&plain));
        // The WAL cost of one isolated update / put, first, then the run.
        let w = written(units.engine()).wal;
        update_one(&units, ids[0], rng.bytes(len))?;
        let one_update = written(units.engine()).wal - w;
        let w = written(&plain).wal;
        put_one(&plain, plain_key(0), rng.bytes(len))?;
        let one_put = written(&plain).wal - w;
        for _ in 1..ops {
            let pick = rng.below(units_n as u64);
            update_one(&units, ids[pick as usize], rng.bytes(len))?;
            put_one(&plain, plain_key(pick), rng.bytes(len))?;
        }
        settle(units.engine())?;
        settle(&plain)?;
        let (wu1, wp1) = (written(units.engine()), written(&plain));
        let logical = (ops * len) as f64;
        let side = |name: &str, w0: Written, w1: Written| {
            let (wal, flush, comp) = (w1.wal - w0.wal, w1.flush - w0.flush, w1.compaction - w0.compaction);
            let total = wal + flush + comp;
            println!(
                "raw U4 {len} B {name}: ops={ops} logical_bytes={} wal_bytes={wal} flush_bytes={flush} \
                 compaction_bytes={comp} total={total} wal_per_op={:.1} total_amp={:.3}",
                logical as u64,
                wal as f64 / ops as f64,
                total as f64 / logical
            );
            (wal as f64 / ops as f64, total as f64 / logical)
        };
        let (uw, ua) = side("update", wu0, wu1);
        let (pw, pa) = side("put", wp0, wp1);
        println!("raw U4 {len} B isolated: wal_bytes update={one_update} put={one_put}");
        verdict(
            out,
            "U4",
            format!(
                "{len} B: WAL/update {uw:.0} B vs WAL/put {pw:.0} B (+{:.0} B, {:.2}x); isolated update {one_update} B \
                 vs put {one_put} B; amplification (WAL+flush+compaction)/payload update {ua:.2} vs put {pa:.2}",
                uw - pw,
                uw / pw
            ),
            None,
        );
        units.close()?;
        plain.close()?;
    }
    Ok(())
}

// ------------------------------------------------------------ U5

fn u5(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    const LEN: usize = 1024;
    println!("## U5: stored bytes per revision after N updates");
    let (du, dp) = (TempDir::new(&a.dir, "u5-units")?, TempDir::new(&a.dir, "u5-plain")?);
    let mut rng = Rng::new(a.seed ^ 0x05);
    let units = open_units(&du.0)?;
    let plain = open_plain(&dp.0)?;
    let ids = create_units(&units, &mut rng, z.u5_units, LEN, 1_000)?;
    add_revisions(&units, &mut rng, &ids, z.u5_revs - 1, LEN, 1_000)?;
    let revisions = (z.u5_units * z.u5_revs) as u64;
    for chunk in (0..revisions).collect::<Vec<_>>().chunks(1_000) {
        let mut b = WriteBatch::new();
        for i in chunk {
            b.put(plain_key(*i), rng.bytes(LEN));
        }
        Engine::commit(&plain, &b)?;
    }
    for db in [units.engine(), &plain] {
        db.compact_range(Bound::Unbounded, Bound::Unbounded)?;
        TestHooks::wait_idle(db)?;
    }
    let (su, sp) = (stored(units.engine()), stored(&plain));
    let (bu, bp) = (su as f64 / revisions as f64, sp as f64 / revisions as f64);
    println!(
        "raw U5: units={} revisions_per_unit={} revisions={revisions} payload={LEN} stored_units={su} \
         stored_plain={sp} per_revision={bu:.1} per_plain_key={bp:.1}",
        z.u5_units, z.u5_revs
    );
    verdict(
        out,
        "U5",
        format!(
            "{revisions} revisions of {LEN} B: {bu:.1} B/revision vs {bp:.1} B per plain key (+{:.1} B, {:.3}x)",
            bu - bp,
            bu / bp
        ),
        None,
    );
    units.close()?;
    plain.close()?;
    Ok(())
}

// ------------------------------------------------------------ U6

#[derive(Default)]
struct Window {
    writes: Vec<u64>,
    reads: Vec<u64>,
    secs: f64,
}

/// Runs the writers and a reader until `stop` while `body` runs, and
/// returns what they measured. `body` returns when the window ends.
fn window(db: &Units, hot: &[UnitId], z: &Sizes, seed: u64, body: impl FnOnce() -> Res<()>) -> Res<Window> {
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| -> Res<Window> {
        let writers: Vec<_> = (0..z.u6_writers)
            .map(|w| {
                let stop = &stop;
                s.spawn(move || -> Result<Vec<u64>, String> {
                    let mut rng = Rng::new(seed ^ (w as u64 + 1));
                    let mut lats = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let id = hot[rng.below(hot.len() as u64) as usize];
                        let payload = rng.bytes(1024);
                        lats.push(timed(|| update_one(db, id, payload)).map_err(|e| e.to_string())?);
                    }
                    Ok(lats)
                })
            })
            .collect();
        let reader = {
            let stop = &stop;
            s.spawn(move || -> Result<Vec<u64>, String> {
                let mut rng = Rng::new(seed ^ 0xEAD);
                let mut lats = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let id = hot[rng.below(hot.len() as u64) as usize];
                    lats.push(timed(|| db.get_unit(id).map(drop).map_err(Into::into)).map_err(|e| e.to_string())?);
                }
                Ok(lats)
            })
        };
        let t = Instant::now();
        let r = body();
        let secs = t.elapsed().as_secs_f64();
        stop.store(true, Ordering::Relaxed);
        let mut w = Window { secs, ..Window::default() };
        for h in writers {
            w.writes.extend(h.join().map_err(|_| "writer panicked")??);
        }
        w.reads = reader.join().map_err(|_| "reader panicked")??;
        r?;
        Ok(w)
    })
}

fn u6(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    // The default budget, then a small one: a Retention change set holds the
    // write slot, so its size bounds what a concurrent writer waits.
    u6_run(a, z, out, RetentionBudget::default())?;
    u6_run(a, z, out, RetentionBudget { max_units: 25, max_ops: 100 })
}

fn u6_run(a: &Args, z: &Sizes, out: &mut Vec<Verdict>, budget: RetentionBudget) -> Res<()> {
    const LEN: usize = 100;
    println!(
        "## U6: run_retention (KeepLast(1), {budget:?}) with {} writer(s) of 1 KiB updates and 1 reader",
        z.u6_writers
    );
    let du = TempDir::new(&a.dir, "u6-units")?;
    let mut rng = Rng::new(a.seed ^ 0x06);
    let units = open_units(&du.0)?;
    let cold = create_units(&units, &mut rng, z.u6_units, LEN, 1_000)?;
    let hot = create_units(&units, &mut rng, z.u6_hot, 1024, 1_000)?;
    let policy = RetentionPolicy::all(RetentionRule::KeepLast(1));
    // Low load: one writer alone, no reader (the T1-like reference).
    let low = {
        let mut lats = Vec::new();
        let end = Instant::now() + z.u6_window;
        let t = Instant::now();
        while Instant::now() < end {
            let id = hot[rng.below(hot.len() as u64) as usize];
            let payload = rng.bytes(1024);
            lats.push(timed(|| update_one(&units, id, payload))?);
        }
        (lat(&lats), lats.len() as f64 / t.elapsed().as_secs_f64())
    };
    show("U6 low load (1 writer, no reader) write", &low.0);
    println!("raw U6 low load throughput_ops_s={:.1}", low.1);
    let (mut a1, mut a2, mut during) = (Window::default(), Window::default(), Window::default());
    let (mut pruned, mut retained, mut rsecs, mut calls) = (0u64, 0u64, 0f64, 0u64);
    for round in 0..z.rounds {
        add_revisions(&units, &mut rng, &cold, z.u6_revs, LEN, 1_000)?;
        settle(units.engine())?;
        let seed = a.seed ^ 0x0600 ^ round as u64;
        let b1 = window(&units, &hot, z, seed, || {
            std::thread::sleep(z.u6_window);
            Ok(())
        })?;
        let b2 = window(&units, &hot, z, seed ^ 1, || {
            std::thread::sleep(z.u6_window);
            Ok(())
        })?;
        let d = window(&units, &hot, z, seed ^ 2, || {
            loop {
                let r = units.run_retention(&policy, budget)?;
                calls += 1;
                pruned += r.revisions_pruned;
                retained += r.units_retained;
                if r.complete {
                    return Ok(());
                }
            }
        })?;
        rsecs += d.secs;
        println!(
            "raw U6 round {round}: before_a {:.2}s {} writes, before_b {:.2}s {} writes, during {:.2}s {} writes",
            b1.secs,
            b1.writes.len(),
            b2.secs,
            b2.writes.len(),
            d.secs,
            d.writes.len()
        );
        for (acc, w) in [(&mut a1, b1), (&mut a2, b2), (&mut during, d)] {
            acc.writes.extend(w.writes);
            acc.reads.extend(w.reads);
            acc.secs += w.secs;
        }
    }
    let mut res = Vec::new();
    for (name, w) in [("before_a", &a1), ("before_b", &a2), ("during", &during)] {
        let (lw, lr) = (lat(&w.writes), lat(&w.reads));
        let thr = w.writes.len() as f64 / w.secs;
        show(&format!("U6 {name} write"), &lw);
        show(&format!("U6 {name} read"), &lr);
        println!("raw U6 {name} write_throughput_ops_s={thr:.1} secs={:.2}", w.secs);
        res.push((lw, lr, thr));
    }
    println!(
        "raw U6 retention: calls={calls} revisions_pruned={pruned} units_retained={retained} secs={rsecs:.3} \
         revisions_per_s={:.0} units_per_s={:.0}",
        pruned as f64 / rsecs,
        retained as f64 / rsecs
    );
    let (b_p99, b_thr) = (res[0].0.p99.max(res[1].0.p99), res[0].2.min(res[1].2));
    let b_rp99 = res[0].1.p99.max(res[1].1.p99);
    let (d_p99, d_thr, d_rp99) = (res[2].0.p99, res[2].2, res[2].1.p99);
    verdict(
        out,
        "U6",
        format!(
            "budget {} units/{} ops: write p99 during {d_p99:.3} ms vs before {b_p99:.3} ms ({:.2}x; low load {:.3} ms); throughput during \
             {d_thr:.1} ops/s vs before {b_thr:.1} ops/s ({:.2}x); read p99 during {d_rp99:.4} ms vs before \
             {b_rp99:.4} ms ({:.2}x); retention {:.0} revisions/s",
            budget.max_units,
            budget.max_ops,
            d_p99 / b_p99,
            low.0.p99,
            d_thr / b_thr,
            d_rp99 / b_rp99,
            pruned as f64 / rsecs
        ),
        Some(d_p99 <= b_p99 && d_thr >= b_thr && d_rp99 <= b_rp99),
    );
    units.close()?;
    Ok(())
}

// ------------------------------------------------------------ main

fn main() -> Res<()> {
    let a = parse_args()?;
    let z = sizes(a.quick);
    println!(
        "# unit_bench mode={} seed={:#x} dir={} engine=KernoDb options={:?} vfs=StdVfs rounds={}",
        if a.quick { "quick" } else { "full" },
        a.seed,
        a.dir.display(),
        options(),
        z.rounds
    );
    let mut out = Vec::new();
    type Bench = fn(&Args, &Sizes, &mut Vec<Verdict>) -> Res<()>;
    let all: [(&str, Bench); 6] = [("U1", u1), ("U2", u2), ("U3", u3), ("U4", u4), ("U5", u5), ("U6", u6)];
    for (id, f) in all {
        if a.only.as_ref().is_none_or(|o| o.iter().any(|x| x == id)) {
            let t = Instant::now();
            f(&a, &z, &mut out)?;
            println!("time {id}: {:.1} s", t.elapsed().as_secs_f64());
        }
    }
    println!("# summary");
    for v in &out {
        let mark = match v.pass {
            Some(true) => "PASS",
            Some(false) => "FAIL",
            None => "INFO",
        };
        println!("{} {mark} {}", v.id, v.text);
    }
    Ok(())
}
