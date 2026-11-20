//! `unit_bench`: the `U` metrics of Stage 3 (stage3-design.md §9.4, WP E3-09,
//! thresholds of decision 12). Every number is measured on `UnitDb<KernoDb>`
//! and on its internal reference, plain keys in `KernoDb`, opened with the
//! same options on the real file system (`StdVfs`), in the same process,
//! with the same warmup; the latency runs alternate reference and layer
//! round by round so both see the same machine load.
//!
//! ```text
//! cargo run --release -p kerno-units --features test-support --example unit_bench            # full run
//! cargo run --release -p kerno-units --features test-support --example unit_bench -- --quick # CI smoke
//! ```
//!
//! Options: `--quick`, `--dir <path>` (default: the system temp dir),
//! `--seed <u64>` (default 0xE309), `--only U1,U3,...`, `--relations`,
//! `--indexes`.
//!
//! `--diag` (Phase C, K-E3-9): U6 also prints what the engine counted in each window (stalls,
//! slowdowns, the transaction gate, bytes flushed and compacted), to find the cause of a verdict.
//! `--budget UNITS,OPS`: U6 runs only with this retention budget (the default is the two budgets
//! of Stage 3), to repeat one of them without waiting for the other.
//! `--control`: after each U6 retention window (and a `settle`), the same writers run for as long
//! again with no retention, to tell what the length of a window costs from what retention costs.
//! It prints and does not change the verdict.
//! `--medians` (row OPS-08, option (a)): U6 runs 7 rounds per budget with the control window on
//! (it implies `--control`) and also judges the medians over the rounds of during / control for
//! write p99, throughput and read p99, against a tolerance taken from the spread between the two
//! "before" windows. The pre-registered rule is in `docs/testing/phase-c-performance.md` §3.8. The
//! verdict of the plain U6 (during against the worse "before" window, pooled rounds) is still
//! printed, with the same thresholds.
//! `--pacing UNITS,RATIO`: every run uses `RetentionPacing { units_per_change_set: UNITS,
//! pause_ratio: RATIO }` instead of the default, for measuring a pacing before changing it.
//!
//! `--indexes` (E5-00, M-9): U6 builds the primary index with
//! `rebuild_index` after loading the data and again at the start of every
//! round, so retention runs over a database that holds index entries.
//!
//! `--relations` (E4-02, K-E3-8 and K-E3-9): U2 and U6 run with declared
//! relations active. The units they create get three outgoing relations each
//! (three records per relation, ADR-0014), and in U6 every writer update also
//! creates one relation in the same change set. Thresholds and method are the
//! same; only the data under the measured operation differs.
//!
//! * U1: p99 of a `Sync` change set with one 1 KiB `update` against the p99
//!   of a `Sync` commit of one 1 KiB put. Threshold <= 1.5x.
//! * U2: p99 of `history(id, latest(100))` (metadata only) against the p99
//!   of a snapshot + `scan` of 100 keys of 100 B. Threshold <= 1.5x. The
//!   lightweight view `history_page` (decision 12c: same checks, no
//!   `Revision` built per entry) is measured after both in each round with
//!   its own random stream and reported, not judged.
//! * U3: p99 of `unit_as_of` on units with 1 000 revisions against the p99 of
//!   a snapshot + `get` of a 1 KiB key. Reported; flagged above 5x.
//! * U4: WAL, flush and compaction bytes per `update` of 1 KiB and 64 KiB
//!   against a plain put of the same value (also the WAL cost of an update
//!   deferred from E3-08). Reported.
//! * U5: stored bytes per revision after N updates, after a full compaction,
//!   against N distinct plain keys of the same size, for payloads of 100 B,
//!   1 KiB and 16 KiB: live table bytes and, after `close`, the bytes of the
//!   directory minus those of an empty database closed the same way. Reported.
//! * U6: writer p99 and throughput (plus a reader's p99) before and during
//!   `run_retention`, with the same concurrent load, and the retention
//!   throughput. Pass: p99 during <= the worse of two "before" windows and
//!   throughput during >= the lower of them.
//! * R1 (row OPS-10): crash-recovery time. A child process (this binary again, through
//!   `kerno_testkit::process`) writes 200 KiB change sets of 1 KiB units until its WAL holds 1,
//!   64 and 256 MiB with nothing flushed, and the parent kills it with `SIGKILL`. The parent then
//!   times `UnitDb::open` (which replays the WAL) up to the first `get_unit` of the last unit the
//!   child committed. 5 repetitions per size, p50 and max. Reported.
//! * T1 (row OPS-07): p50 and p99 of `unit_as_of` at a depth of 10, 100 and
//!   1000 revisions of 1 KiB, the target drawn below the head so that every
//!   read takes the binary search over `REV_META`; each read must return the
//!   revision in force. Reported.
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
    AsOf, ChangeMeta, CommitSeq, CounterIds, FixedClock, HistoryQuery, NewUnit, RelationProvenance, RetentionBudget,
    RetentionPolicy, RetentionRule, RevNo, Timestamp, UnitAt, UnitConfig, UnitDb, UnitId, UnitUpdate,
};

type Res<T> = Result<T, Box<dyn std::error::Error>>;
type Units = UnitDb<KernoDb>;

// ------------------------------------------------------------ setup

struct Args {
    quick: bool,
    dir: PathBuf,
    seed: u64,
    only: Option<Vec<String>>,
    relations: bool,
    indexes: bool,
    diag: bool,
    control: bool,
    medians: bool,
    budget: Option<RetentionBudget>,
}

/// `--pacing`, read by [`unit_config`].
static PACING: std::sync::OnceLock<kerno_units::RetentionPacing> = std::sync::OnceLock::new();

fn parse_args() -> Res<Args> {
    let mut a = Args {
        quick: false,
        dir: std::env::temp_dir(),
        seed: 0xE309,
        only: None,
        relations: false,
        indexes: false,
        diag: false,
        control: false,
        medians: false,
        budget: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--quick" => a.quick = true,
            "--relations" => a.relations = true,
            "--indexes" => a.indexes = true,
            "--diag" => a.diag = true,
            "--control" => a.control = true,
            "--medians" => a.medians = true,
            "--budget" => {
                let v = it.next().ok_or("--budget needs UNITS,OPS")?;
                let (units, ops) = v.split_once(',').ok_or("--budget needs UNITS,OPS")?;
                a.budget = Some(RetentionBudget { max_units: units.parse()?, max_ops: ops.parse()? });
            }
            "--pacing" => {
                let v = it.next().ok_or("--pacing needs UNITS,RATIO")?;
                let (units, ratio) = v.split_once(',').ok_or("--pacing needs UNITS,RATIO")?;
                let pacing =
                    kerno_units::RetentionPacing { units_per_change_set: units.parse()?, pause_ratio: ratio.parse()? };
                PACING.set(pacing).map_err(|_| "--pacing given twice")?;
            }
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
    if let Some(p) = PACING.get() {
        c.retention_pacing = *p;
    }
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

/// What the counters and the settle step need, for a bare engine and for the
/// layer over one, so a measurement reads both through the same code.
trait Measured {
    fn counters(&self) -> Vec<(String, u64)>;
    fn settle(&self) -> Res<()>;
}

impl Measured for KernoDb {
    fn counters(&self) -> Vec<(String, u64)> {
        Engine::stats(self)
    }
    fn settle(&self) -> Res<()> {
        TestHooks::flush(self)?;
        TestHooks::wait_idle(self)?;
        Ok(())
    }
}

impl Measured for UnitDb<KernoDb> {
    fn counters(&self) -> Vec<(String, u64)> {
        self.storage_stats()
    }
    fn settle(&self) -> Res<()> {
        self.hook_flush()?;
        self.hook_wait_idle()?;
        Ok(())
    }
}

fn stat(db: &impl Measured, name: &str) -> u64 {
    db.counters().into_iter().find(|(n, _)| n == name).map_or(0, |(_, v)| v)
}

/// Bytes the engine wrote: WAL, flushes, compactions.
#[derive(Clone, Copy, Debug, Default)]
struct Written {
    wal: u64,
    flush: u64,
    compaction: u64,
}

fn written(db: &impl Measured) -> Written {
    Written {
        wal: stat(db, "wal_bytes"),
        flush: stat(db, "flush_bytes_written"),
        compaction: stat(db, "compaction_bytes_written"),
    }
}

/// Live table bytes (sum of `l<N>_bytes`).
fn stored(db: &impl Measured) -> u64 {
    let level = |n: &str| {
        n.strip_prefix('l')
            .and_then(|r| r.strip_suffix("_bytes"))
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    };
    db.counters().into_iter().filter(|(n, _)| level(n)).map(|(_, v)| v).sum()
}

/// Bytes of the files directly in `dir` (the engine directory is flat).
fn dir_bytes(dir: &Path) -> Res<u64> {
    Ok(kerno_core::vfs::dir_size(&*StdVfs::shared(), dir)?)
}

/// The bytes U5 subtracts so that its on-disk figure is per revision, not per database.
fn empty_dir_bytes(a: &Args) -> Res<(u64, u64)> {
    let (du, dp) = (TempDir::new(&a.dir, "u5-empty-units")?, TempDir::new(&a.dir, "u5-empty-plain")?);
    open_units(&du.0)?.close()?;
    open_plain(&dp.0)?.close()?;
    Ok((dir_bytes(&du.0)?, dir_bytes(&dp.0)?))
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

/// `update_one` plus one declared relation `id -> other`, in the same change set.
fn update_and_relate(db: &Units, id: UnitId, other: UnitId, payload: Vec<u8>) -> Res<()> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    cs.update(id, upd(payload), None)?;
    cs.create_relation(id, other, "bench", RelationProvenance::Declared)?;
    cs.commit()?;
    Ok(())
}

/// Gives every unit of `ids` `per_unit` outgoing relations to random units of
/// `ids`, `per_cs` relations per change set.
fn add_relations(db: &Units, rng: &mut Rng, ids: &[UnitId], per_unit: usize, per_cs: usize) -> Res<()> {
    let total = ids.len() * per_unit;
    let mut made = 0;
    while made < total {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        for _ in 0..per_cs.min(total - made) {
            let from = ids[made % ids.len()];
            let to = ids[rng.below(ids.len() as u64) as usize];
            cs.create_relation(from, to, "bench", RelationProvenance::Declared)?;
            made += 1;
        }
        cs.commit()?;
    }
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
fn settle(db: &impl Measured) -> Res<()> {
    db.settle()
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
    t1_units: usize,
    t1_queries: usize,
    u6_units: usize,
    u6_revs: usize,
    u6_hot: usize,
    u6_writers: usize,
    u6_window: Duration,
    r1_mib: &'static [u64],
    r1_reps: usize,
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
            t1_units: 1,
            t1_queries: 200,
            u6_units: 500,
            u6_revs: 5,
            u6_hot: 10,
            u6_writers: 2,
            u6_window: Duration::from_millis(300),
            r1_mib: &[1, 4],
            r1_reps: 2,
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
            t1_units: 10,
            t1_queries: 5_000,
            u6_units: 10_000,
            u6_revs: 20,
            u6_hot: 100,
            u6_writers: 2,
            u6_window: Duration::from_secs(3),
            r1_mib: &[1, 64, 256],
            r1_reps: 5,
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
    if a.relations {
        add_relations(&units, &mut rng, &ids, 3, 1_000)?;
    }
    let keys = (z.u2_units * REVS) as u64;
    for chunk in (0..keys).collect::<Vec<_>>().chunks(1_000) {
        let mut b = WriteBatch::new();
        for i in chunk {
            b.put(plain_key(*i), rng.bytes(LEN));
        }
        Engine::commit(&plain, &b)?;
    }
    settle(&units)?;
    settle(&plain)?;
    // Judged: `history` against the reference scan, alternated round by
    // round, same random stream as before decision 12c. `history_page`
    // (decision 12c) is measured after both in each round, with its own
    // random stream, and reported, not judged.
    let mut rv = Rng::new(a.seed ^ 0x2C);
    let (mut su, mut sp, mut sv) = (Vec::new(), Vec::new(), Vec::new());
    let warm = z.u2_queries / 10;
    for round in 0..z.rounds {
        for side in [round % 2, 1 - round % 2, 2] {
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
                } else if side == 2 {
                    let id = ids[rv.below(ids.len() as u64) as usize];
                    timed(|| {
                        let h = units.history_page(id, HistoryQuery::latest(REVS))?;
                        if h.len() != REVS {
                            return Err(format!("history_page returned {} revisions", h.len()).into());
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
                    match side {
                        0 => su.push(ns),
                        1 => sp.push(ns),
                        _ => sv.push(ns),
                    }
                }
            }
        }
    }
    let (lu, lp, lv) = (lat(&su), lat(&sp), lat(&sv));
    show("U2 history", &lu);
    show("U2 scan", &lp);
    show("U2 history_page (informative)", &lv);
    let ratio = lu.p99 / lp.p99;
    verdict(
        out,
        "U2",
        format!(
            "p99 history {:.4} ms / scan {:.4} ms = {ratio:.2}x (threshold <= 1.5x); history_page {:.4} ms = {:.2}x (informative)",
            lu.p99,
            lp.p99,
            lv.p99,
            lv.p99 / lp.p99
        ),
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
    let first = units.raw_seq()? + 1;
    let ids = create_units(&units, &mut rng, z.u3_units, LEN, 1_000)?;
    add_revisions(&units, &mut rng, &ids, z.u3_revs - 1, LEN, 1_000)?;
    let last = units.raw_seq()?;
    let keys = (z.u3_units * z.u3_revs) as u64;
    for chunk in (0..keys).collect::<Vec<_>>().chunks(1_000) {
        let mut b = WriteBatch::new();
        for i in chunk {
            b.put(plain_key(*i), rng.bytes(LEN));
        }
        Engine::commit(&plain, &b)?;
    }
    settle(&units)?;
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
        settle(&units)?;
        settle(&plain)?;
        let (wu0, wp0) = (written(&units), written(&plain));
        // The WAL cost of one isolated update / put, first, then the run.
        let w = written(&units).wal;
        update_one(&units, ids[0], rng.bytes(len))?;
        let one_update = written(&units).wal - w;
        let w = written(&plain).wal;
        put_one(&plain, plain_key(0), rng.bytes(len))?;
        let one_put = written(&plain).wal - w;
        for _ in 1..ops {
            let pick = rng.below(units_n as u64);
            update_one(&units, ids[pick as usize], rng.bytes(len))?;
            put_one(&plain, plain_key(pick), rng.bytes(len))?;
        }
        settle(&units)?;
        settle(&plain)?;
        let (wu1, wp1) = (written(&units), written(&plain));
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
    println!("## U5: stored bytes per revision after N updates");
    let (eu, ep) = empty_dir_bytes(a)?;
    for len in [100usize, 1024, 16 * 1024] {
        let (du, dp) =
            (TempDir::new(&a.dir, &format!("u5-units-{len}"))?, TempDir::new(&a.dir, &format!("u5-plain-{len}"))?);
        let mut rng = Rng::new(a.seed ^ 0x05);
        let units = open_units(&du.0)?;
        let plain = open_plain(&dp.0)?;
        let ids = create_units(&units, &mut rng, z.u5_units, len, 1_000)?;
        add_revisions(&units, &mut rng, &ids, z.u5_revs - 1, len, 1_000)?;
        let revisions = (z.u5_units * z.u5_revs) as u64;
        for chunk in (0..revisions).collect::<Vec<_>>().chunks(1_000) {
            let mut b = WriteBatch::new();
            for i in chunk {
                b.put(plain_key(*i), rng.bytes(len));
            }
            Engine::commit(&plain, &b)?;
        }
        units.hook_compact_range(Bound::Unbounded, Bound::Unbounded)?;
        units.hook_wait_idle()?;
        plain.compact_range(Bound::Unbounded, Bound::Unbounded)?;
        TestHooks::wait_idle(&plain)?;
        let (su, sp) = (stored(&units), stored(&plain));
        let (bu, bp) = (su as f64 / revisions as f64, sp as f64 / revisions as f64);
        units.close()?;
        plain.close()?;
        let (fu, fp) = (dir_bytes(&du.0)?, dir_bytes(&dp.0)?);
        let (du_rev, dp_key) =
            (fu.saturating_sub(eu) as f64 / revisions as f64, fp.saturating_sub(ep) as f64 / revisions as f64);
        println!(
            "raw U5: units={} revisions_per_unit={} revisions={revisions} payload={len} stored_units={su} \
             stored_plain={sp} per_revision={bu:.1} per_plain_key={bp:.1} dir_units={fu} dir_plain={fp} \
             dir_empty_units={eu} dir_empty_plain={ep} disk_per_revision={du_rev:.1} disk_per_plain_key={dp_key:.1}",
            z.u5_units, z.u5_revs
        );
        verdict(
            out,
            "U5",
            format!(
                "{revisions} revisions of {len} B: {bu:.1} B/revision vs {bp:.1} B per plain key (+{:.1} B, {:.3}x); \
                 on disk after close {du_rev:.1} B/revision vs {dp_key:.1} B per plain key ({:.3}x)",
                bu - bp,
                bu / bp,
                du_rev / dp_key
            ),
            None,
        );
    }
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
fn window(
    db: &Units,
    hot: &[UnitId],
    z: &Sizes,
    relate: bool,
    seed: u64,
    body: impl FnOnce() -> Res<()>,
) -> Res<Window> {
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
                        let other = hot[rng.below(hot.len() as u64) as usize];
                        let op = || {
                            if relate { update_and_relate(db, id, other, payload) } else { update_one(db, id, payload) }
                        };
                        lats.push(timed(op).map_err(|e| e.to_string())?);
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

/// `--diag`: what the engine counted while a window ran.
fn diag_window(a: &Args, label: &str, before: &[(String, u64)], db: &Units) {
    if !a.diag {
        return;
    }
    const KEYS: [&str; 7] =
        ["stall", "slowdown", "txn_gate", "flush_bytes_written", "compaction_bytes_written", "wal_syncs", "flushes"];
    let delta: Vec<String> = db
        .counters()
        .into_iter()
        .filter(|(n, _)| KEYS.iter().any(|k| n.contains(k)))
        .map(|(n, v)| {
            let was = before.iter().find(|(m, _)| *m == n).map_or(0, |(_, b)| *b);
            format!("{n}={}", v.saturating_sub(was))
        })
        .filter(|d| !d.ends_with("=0"))
        .collect();
    println!("raw U6 diag {label}: {}", delta.join(" "));
}

/// `--medians`: rounds per budget (pre-registered in `docs/testing/phase-c-performance.md` §3.8).
const MEDIAN_ROUNDS: usize = 7;
/// Smallest tolerances, whatever the "before" windows spread: the 2 % of throughput that
/// §3.4 of the report measured between two "before" windows, and 10 % of p99.
const MIN_TOL_THROUGHPUT: f64 = 0.02;
const MIN_TOL_P99: f64 = 0.10;

/// What one window of one round contributes to the medians.
#[derive(Clone, Copy)]
struct WindowStats {
    throughput: f64,
    write_p99: f64,
    read_p99: f64,
}

impl WindowStats {
    fn of(w: &Window) -> Self {
        WindowStats {
            throughput: w.writes.len() as f64 / w.secs,
            write_p99: lat(&w.writes).p99,
            read_p99: lat(&w.reads).p99,
        }
    }
}

struct RoundStats {
    before: [WindowStats; 2],
    during: WindowStats,
    control: WindowStats,
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Relative gap between two "before" windows: |a - b| / max(a, b).
fn spread(a: f64, b: f64) -> f64 {
    (a - b).abs() / a.max(b)
}

/// The pre-registered statistic of `--medians`: for write p99, throughput and read p99, the
/// median over the rounds of during / control; the noise is the median over the rounds of the
/// spread between the two "before" windows, floored at [`MIN_TOL_THROUGHPUT`] and [`MIN_TOL_P99`].
/// Fails when the throughput median is below `1 - tolerance`, or a p99 median is above
/// `1 + tolerance`. The fixed reading of the plan (0.98 and 1.10, write p99 and throughput only)
/// is printed beside it.
fn medians_verdict(out: &mut Vec<Verdict>, budget: RetentionBudget, rounds: &[RoundStats]) {
    let per = |f: fn(&WindowStats) -> f64| {
        let ratio = median(rounds.iter().map(|r| f(&r.during) / f(&r.control)).collect());
        let noise = median(rounds.iter().map(|r| spread(f(&r.before[0]), f(&r.before[1]))).collect());
        (ratio, noise)
    };
    let (thr, thr_noise) = per(|w| w.throughput);
    let (wp99, wp99_noise) = per(|w| w.write_p99);
    let (rp99, rp99_noise) = per(|w| w.read_p99);
    for (i, r) in rounds.iter().enumerate() {
        println!(
            "raw U6 median round {i}: during/control throughput {:.3} write_p99 {:.3} read_p99 {:.3}; \
             before_a/before_b spread throughput {:.3} write_p99 {:.3} read_p99 {:.3}",
            r.during.throughput / r.control.throughput,
            r.during.write_p99 / r.control.write_p99,
            r.during.read_p99 / r.control.read_p99,
            spread(r.before[0].throughput, r.before[1].throughput),
            spread(r.before[0].write_p99, r.before[1].write_p99),
            spread(r.before[0].read_p99, r.before[1].read_p99),
        );
    }
    let (t_thr, t_wp99, t_rp99) =
        (thr_noise.max(MIN_TOL_THROUGHPUT), wp99_noise.max(MIN_TOL_P99), rp99_noise.max(MIN_TOL_P99));
    let pass = thr >= 1.0 - t_thr && wp99 <= 1.0 + t_wp99 && rp99 <= 1.0 + t_rp99;
    let fixed = thr >= 0.98 && wp99 <= 1.10;
    verdict(
        out,
        "U6m",
        format!(
            "budget {} units/{} ops, median of {} rounds of during/control: throughput {thr:.3} (noise {thr_noise:.3}, \
             fails below {:.3}); write p99 {wp99:.3} (noise {wp99_noise:.3}, fails above {:.3}); read p99 {rp99:.3} \
             (noise {rp99_noise:.3}, fails above {:.3}); fixed 0.98/1.10 rule: {}",
            budget.max_units,
            budget.max_ops,
            rounds.len(),
            1.0 - t_thr,
            1.0 + t_wp99,
            1.0 + t_rp99,
            if fixed { "PASS" } else { "FAIL" }
        ),
        Some(pass),
    );
}

fn u6(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    // The default budget, then a small one: a Retention change set holds the
    // write slot, so its size bounds what a concurrent writer waits.
    if let Some(budget) = a.budget {
        return u6_run(a, z, out, budget);
    }
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
    if a.relations {
        add_relations(&units, &mut rng, &cold, 3, 1_000)?;
        add_relations(&units, &mut rng, &hot, 3, 1_000)?;
    }
    if a.indexes {
        units.rebuild_index()?;
    }
    let policy = RetentionPolicy::all(RetentionRule::KeepLast(1));
    // Low load: one writer alone, no reader (the T1-like reference).
    let low = {
        let mut lats = Vec::new();
        let end = Instant::now() + z.u6_window;
        let t = Instant::now();
        while Instant::now() < end {
            let id = hot[rng.below(hot.len() as u64) as usize];
            let payload = rng.bytes(1024);
            let other = hot[rng.below(hot.len() as u64) as usize];
            lats.push(timed(|| {
                if a.relations {
                    update_and_relate(&units, id, other, payload)
                } else {
                    update_one(&units, id, payload)
                }
            })?);
        }
        (lat(&lats), lats.len() as f64 / t.elapsed().as_secs_f64())
    };
    show("U6 low load (1 writer, no reader) write", &low.0);
    println!("raw U6 low load throughput_ops_s={:.1}", low.1);
    let (mut a1, mut a2, mut during, mut ctl) =
        (Window::default(), Window::default(), Window::default(), Window::default());
    let (mut pruned, mut retained, mut rsecs, mut calls) = (0u64, 0u64, 0f64, 0u64);
    let control = a.control || a.medians;
    let rounds = if a.medians { MEDIAN_ROUNDS } else { z.rounds };
    let mut per_round = Vec::new();
    for round in 0..rounds {
        add_revisions(&units, &mut rng, &cold, z.u6_revs, LEN, 1_000)?;
        if a.indexes {
            units.rebuild_index()?;
        }
        settle(&units)?;
        let seed = a.seed ^ 0x0600 ^ round as u64;
        let c = units.counters();
        let b1 = window(&units, &hot, z, a.relations, seed, || {
            std::thread::sleep(z.u6_window);
            Ok(())
        })?;
        diag_window(a, &format!("round {round} before_a"), &c, &units);
        let c = units.counters();
        let b2 = window(&units, &hot, z, a.relations, seed ^ 1, || {
            std::thread::sleep(z.u6_window);
            Ok(())
        })?;
        diag_window(a, &format!("round {round} before_b"), &c, &units);
        let c = units.counters();
        let d = window(&units, &hot, z, a.relations, seed ^ 2, || {
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
        diag_window(a, &format!("round {round} during"), &c, &units);
        rsecs += d.secs;
        if control {
            // Same writers for as long as retention took, with no retention,
            // once the engine has absorbed what the window wrote: a window of
            // that length with nothing but writers in it.
            settle(&units)?;
            let c = units.counters();
            let ctl_secs = d.secs;
            let k = window(&units, &hot, z, a.relations, seed ^ 3, || {
                std::thread::sleep(Duration::from_secs_f64(ctl_secs));
                Ok(())
            })?;
            diag_window(a, &format!("round {round} control"), &c, &units);
            println!("raw U6 round {round} control: {:.2}s {} writes", k.secs, k.writes.len());
            if a.medians {
                per_round.push(RoundStats {
                    before: [WindowStats::of(&b1), WindowStats::of(&b2)],
                    during: WindowStats::of(&d),
                    control: WindowStats::of(&k),
                });
            }
            ctl.writes.extend(k.writes);
            ctl.reads.extend(k.reads);
            ctl.secs += k.secs;
        }
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
    if control {
        let (lw, lr) = (lat(&ctl.writes), lat(&ctl.reads));
        show("U6 control (no retention, as long as during) write", &lw);
        show("U6 control read", &lr);
        println!(
            "raw U6 control write_throughput_ops_s={:.1} secs={:.2}",
            ctl.writes.len() as f64 / ctl.secs,
            ctl.secs
        );
    }
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
    if a.medians {
        medians_verdict(out, budget, &per_round);
    }
    units.close()?;
    Ok(())
}

// ------------------------------------------------------------ main

// ------------------------------------------------------------ R1

/// Write buffer of the R1 databases: large enough that nothing flushes before the WAL holds the
/// biggest size, so the whole WAL is replayed by the recovery that is timed.
const R1_WRITE_BUFFER: usize = 1 << 30;

fn r1_open(dir: &Path) -> Res<Units> {
    let mut c = unit_config();
    c.engine.write_buffer_bytes = R1_WRITE_BUFFER;
    Ok(Units::open(StdVfs::shared(), dir, &c)?)
}

/// The child of R1 (this binary run again, see `main`): writes until its WAL holds `mib` MiB with
/// nothing flushed, tells the parent, and waits to be killed.
fn r1_child() -> Res<()> {
    use kerno_testkit::process::{child_args, report};
    let args = child_args();
    let (dir, mib) = (
        PathBuf::from(args.first().ok_or("R1 child needs a dir")?),
        args.get(1).ok_or("R1 child needs a size")?.parse::<u64>()?,
    );
    let units = r1_open(&dir)?;
    let mut rng = Rng::new(0xE309 ^ 0xA1);
    let mut last = None;
    while stat(&units, "wal_bytes") < mib << 20 {
        last = create_units(&units, &mut rng, 200, 1024, 200)?.pop();
    }
    let id = last.ok_or("R1 child wrote nothing")?;
    if stat(&units, "flush_bytes_written") != 0 {
        return Err("R1 child flushed: the WAL is not all unflushed".into());
    }
    report(&format!("ready wal_bytes={} id={id}", stat(&units, "wal_bytes")));
    // The parent kills this process; a bounded wait keeps a lost parent from leaving it behind.
    std::thread::sleep(Duration::from_secs(600));
    Ok(())
}

fn parse_unit_id(hex: &str) -> Res<UnitId> {
    if hex.len() != 32 {
        return Err(format!("unit id {hex:?} is not 32 hex digits").into());
    }
    let mut b = [0u8; 16];
    for (i, byte) in b.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)?;
    }
    Ok(UnitId(b))
}

fn r1(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    use kerno_testkit::process::{next_report, spawn_child};
    println!("## R1 recovery: kill -9 with N MiB of WAL unflushed, then open and read ({} repetitions)", z.r1_reps);
    let mut lines = Vec::new();
    for &mib in z.r1_mib {
        let (mut totals, mut opens) = (Vec::new(), Vec::new());
        for rep in 0..z.r1_reps {
            let dir = TempDir::new(&a.dir, "r1")?;
            let dir_arg = dir.0.to_str().ok_or("non-UTF-8 directory")?;
            let mut child = spawn_child("", "r1", &[dir_arg, &mib.to_string()]);
            let mut stdout = std::io::BufReader::new(child.stdout.take().expect("spawn_child pipes stdout"));
            let ready = next_report(&mut stdout);
            let _ = child.kill(); // SIGKILL; an error means it already ended, `wait` reaps it
            child.wait()?;
            let ready = ready.ok_or("the R1 child ended before it was ready")?;
            let (wal, id) = ready
                .strip_prefix("ready wal_bytes=")
                .and_then(|r| r.split_once(" id="))
                .ok_or_else(|| format!("unexpected report {ready:?}"))?;
            let id = parse_unit_id(id)?;
            let t = Instant::now();
            let units = r1_open(&dir.0)?;
            let open_ns = t.elapsed().as_nanos() as u64;
            units.get_unit(id)?;
            let total_ns = t.elapsed().as_nanos() as u64;
            println!(
                "raw R1 size_mib={mib} rep={rep} wal_bytes={wal} open_ms={:.1} open_and_first_read_ms={:.1}",
                open_ns as f64 / 1e6,
                total_ns as f64 / 1e6
            );
            drop(units);
            opens.push(open_ns);
            totals.push(total_ns);
        }
        let l = lat(&totals);
        show(&format!("R1 {mib} MiB open + first read"), &l);
        lines.push(format!(
            "{mib} MiB: p50 {:.1} ms, max {:.1} ms (open alone p50 {:.1} ms)",
            l.p50,
            l.max,
            lat(&opens).p50
        ));
    }
    verdict(out, "R1", lines.join("; "), None);
    Ok(())
}

// ------------------------------------------------------------ T1

fn t1(a: &Args, z: &Sizes, out: &mut Vec<Verdict>) -> Res<()> {
    const LEN: usize = 1024;
    const DEPTHS: [usize; 3] = [10, 100, 1_000];
    println!("## T1 as-of: unit_as_of at a depth of 10, 100 and 1000 revisions (1 KiB payloads)");
    let mut dbs = Vec::new();
    for d in DEPTHS {
        let dir = TempDir::new(&a.dir, &format!("t1-{d}"))?;
        let units = open_units(&dir.0)?;
        let mut rng = Rng::new(a.seed ^ 0x7100 ^ d as u64);
        let mut seqs = Vec::with_capacity(d);
        let mut cs = units.begin_changeset(ChangeMeta::default())?;
        let ids: Vec<UnitId> =
            (0..z.t1_units).map(|_| cs.create(new_unit(rng.bytes(LEN)))).collect::<Result<_, _>>()?;
        seqs.push(cs.commit()?.commit_seq);
        for _ in 1..d {
            let mut cs = units.begin_changeset(ChangeMeta::default())?;
            for id in &ids {
                cs.update(*id, upd(rng.bytes(LEN)), None)?;
            }
            seqs.push(cs.commit()?.commit_seq);
        }
        // Every unit is in every change set, so seqs[r - 1] is the commit of revision r.
        settle(&units)?;
        dbs.push((dir, units, ids, seqs));
    }
    let mut rng = Rng::new(a.seed ^ 0x7101);
    let warm = z.t1_queries / 10;
    let mut samples: Vec<Vec<u64>> = vec![Vec::new(); DEPTHS.len()];
    for round in 0..z.rounds {
        for k in 0..DEPTHS.len() {
            let di = (round + k) % DEPTHS.len();
            let d = DEPTHS[di];
            let (_, units, ids, seqs) = &dbs[di];
            for i in 0..warm + z.t1_queries {
                let id = ids[rng.below(ids.len() as u64) as usize];
                // Below the head, so every read takes the binary search.
                let r = rng.range(1, d as u64 - 1);
                let at = seqs[r as usize - 1];
                let ns = timed(|| match units.unit_as_of(id, AsOf::Seq(at))? {
                    UnitAt::Present(rev) if rev.id.rev == RevNo(r) => Ok(()),
                    _ => Err(format!("T1 depth {d}: unit_as_of did not return revision {r}").into()),
                })?;
                if i >= warm {
                    samples[di].push(ns);
                }
            }
        }
    }
    let ls: Vec<Lat> = samples.iter().map(|s| lat(s)).collect();
    for (d, l) in DEPTHS.iter().zip(&ls) {
        show(&format!("T1 unit_as_of depth {d}"), l);
    }
    verdict(
        out,
        "T1",
        format!(
            "unit_as_of p50/p99: depth 10 {:.4}/{:.4} ms, depth 100 {:.4}/{:.4} ms, depth 1000 {:.4}/{:.4} ms (report)",
            ls[0].p50, ls[0].p99, ls[1].p50, ls[1].p99, ls[2].p50, ls[2].p99
        ),
        None,
    );
    for (_, units, _, _) in dbs {
        units.close()?;
    }
    Ok(())
}

fn main() -> Res<()> {
    // R1 starts this binary again as the writer it kills (`kerno_testkit::process`).
    if kerno_testkit::process::child_role().as_deref() == Some("r1") {
        return r1_child();
    }
    let a = parse_args()?;
    let z = sizes(a.quick);
    println!(
        "# unit_bench mode={} relations={} indexes={} seed={:#x} dir={} engine=KernoDb options={:?} vfs=StdVfs rounds={}",
        if a.quick { "quick" } else { "full" },
        a.relations,
        a.indexes,
        a.seed,
        a.dir.display(),
        options(),
        z.rounds
    );
    let mut out = Vec::new();
    type Bench = fn(&Args, &Sizes, &mut Vec<Verdict>) -> Res<()>;
    let all: [(&str, Bench); 8] =
        [("U1", u1), ("U2", u2), ("U3", u3), ("U4", u4), ("U5", u5), ("U6", u6), ("T1", t1), ("R1", r1)];
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
