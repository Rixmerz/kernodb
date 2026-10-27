//! Benchmark workloads. Each one starts from an empty directory and reports
//! a flat set of metrics. Three are shaped after Kerno's reference use cases
//! (LiveSpec, Cauce, agent memory); the rest are classic micro-benchmarks.

use std::collections::VecDeque;
use std::ops::Bound;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use spike_core::rng::Rng;
use spike_core::vfs::{CountingVfs, IoCounters, IoSnapshot, StdVfs};
use spike_core::{Durability, Engine, Options, Result, Snapshot, WriteBatch};

use crate::metrics::{dir_size, proc_io, Lat, Report};

/// M0 workloads (SAD §17.3) followed by the reference use cases (SAD §1.3).
pub const WORKLOADS: &[&str] = &[
    "bulk_load",
    "point_reads",
    "range_scans",
    "small_commits",
    "append_seq",
    "mixed",
    "history_sim",
    "livespec",
    "cauce",
    "agent_memory",
];

pub struct Ctx {
    pub dir: PathBuf,
    pub opts: Options,
    /// Number of records in the base data set.
    pub n: u64,
    pub seed: u64,
}

struct Db<E: Engine> {
    db: Option<E>,
    io: Arc<IoCounters>,
}

impl<E: Engine> Db<E> {
    fn open(ctx: &Ctx) -> Result<Self> {
        let (vfs, io) = CountingVfs::wrap(StdVfs::shared());
        Ok(Db { db: Some(E::open(vfs, &ctx.dir, &ctx.opts)?), io })
    }

    fn get(&self) -> &E {
        self.db.as_ref().unwrap()
    }

    fn close(&mut self) -> Result<()> {
        self.db.take().unwrap().close()
    }

    fn reopen(&mut self, ctx: &Ctx) -> Result<()> {
        let (vfs, io) = CountingVfs::wrap(StdVfs::shared());
        self.db = Some(E::open(vfs, &ctx.dir, &ctx.opts)?);
        self.io = io;
        Ok(())
    }
}

/// Bijective mix so that `user_key(i)` visits the key space in random order.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Random value with a length uniform in `lo..=hi`.
fn val(rng: &mut Rng, lo: u64, hi: u64) -> Vec<u8> {
    let len = rng.range(lo, hi) as usize;
    rng.bytes(len)
}

fn user_key(i: u64) -> Vec<u8> {
    format!("user{:016x}", mix(i)).into_bytes()
}

/// Loads `n` random-order records with values of `lo..=hi` bytes in batches
/// of 1000. Returns logical bytes written.
fn load<E: Engine>(db: &E, n: u64, rng: &mut Rng, lo: u64, hi: u64) -> Result<u64> {
    let mut logical = 0;
    let mut b = WriteBatch::new();
    for i in 0..n {
        let v = val(rng, lo, hi);
        b.put(user_key(i), v);
        if b.len() == 1000 || i + 1 == n {
            logical += b.payload_bytes() as u64;
            db.commit(&b)?;
            b = WriteBatch::new();
        }
    }
    Ok(logical)
}

pub fn run<E: Engine>(workload: &str, ctx: &Ctx, rep: &mut Report) -> Result<()> {
    match workload {
        "bulk_load" => bulk_load::<E>(ctx, rep),
        "point_reads" => point_reads::<E>(ctx, rep),
        "range_scans" => range_scans::<E>(ctx, rep),
        "small_commits" => small_commits::<E>(ctx, rep),
        "append_seq" => append_seq::<E>(ctx, rep),
        "mixed" => mixed::<E>(ctx, rep),
        "history_sim" => history_sim::<E>(ctx, rep),
        "livespec" => livespec::<E>(ctx, rep),
        "cauce" => cauce::<E>(ctx, rep),
        "agent_memory" => agent_memory::<E>(ctx, rep),
        other => panic!("unknown workload {other}"),
    }
}

fn io_report(rep: &mut Report, prefix: &str, s: &IoSnapshot, commits: u64) {
    if s.file_syncs + s.dir_syncs > 0 && commits > 0 {
        rep.num(&format!("{prefix}_fsyncs_per_commit"), (s.file_syncs + s.dir_syncs) as f64 / commits as f64);
    }
}

/// Random-order bulk load, then clean close and reopen.
fn bulk_load<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let mut db = Db::<E>::open(ctx)?;
    let (_, w0) = proc_io();
    let t = Instant::now();
    let logical = load(db.get(), ctx.n, &mut rng, 100, 1000)?;
    let secs = t.elapsed().as_secs_f64();
    let (_, w1) = proc_io();
    rep.num("load_s", secs);
    rep.num("load_kops", ctx.n as f64 / secs / 1000.0);
    rep.num("load_mb_s", logical as f64 / secs / 1e6);
    rep.num("logical_mb", logical as f64 / 1e6);
    rep.num("write_amp", (w1 - w0) as f64 / logical as f64);
    rep.num("space_amp_open", dir_size(&ctx.dir) as f64 / logical as f64);
    let t = Instant::now();
    db.close()?;
    rep.num("close_ms", t.elapsed().as_secs_f64() * 1000.0);
    rep.num("space_amp_closed", dir_size(&ctx.dir) as f64 / logical as f64);
    let t = Instant::now();
    db.reopen(ctx)?;
    let s = db.get().snapshot()?;
    assert!(s.get(&user_key(ctx.n / 2))?.is_some(), "loaded key missing after reopen");
    rep.num("reopen_ms", t.elapsed().as_secs_f64() * 1000.0);
    drop(s);
    // Unclean shutdown after a burst of small commits, then recovery.
    for i in 0..1000 {
        let mut b = WriteBatch::new();
        b.put(user_key(i), rng.bytes(200));
        db.get().commit(&b)?;
    }
    drop(db.db.take());
    let t = Instant::now();
    db.reopen(ctx)?;
    assert!(db.get().snapshot()?.get(&user_key(999))?.is_some());
    rep.num("recovery_ms", t.elapsed().as_secs_f64() * 1000.0);
    db.close()
}

fn preload_and_reopen<E: Engine>(ctx: &Ctx, rng: &mut Rng, rep: &mut Report) -> Result<Db<E>> {
    let mut db = Db::<E>::open(ctx)?;
    load(db.get(), ctx.n, rng, 100, 1000)?;
    db.close()?;
    let t = Instant::now();
    db.reopen(ctx)?;
    rep.num("reopen_ms", t.elapsed().as_secs_f64() * 1000.0);
    Ok(db)
}

/// Random point reads (90% hits) on one thread, then on four.
fn point_reads<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let db = preload_and_reopen::<E>(ctx, &mut rng, rep)?;
    let n = ctx.n;
    let one = |rng: &mut Rng, lat: &mut Lat, s: &E::Snapshot| -> Result<()> {
        let hit = rng.chance(9, 10);
        let k = if hit { user_key(rng.below(n)) } else { user_key(n + rng.below(n)) };
        let got = lat.time(|| s.get(&k))?;
        assert_eq!(got.is_some(), hit);
        Ok(())
    };
    let s = db.get().snapshot()?;
    let mut lat = Lat::new();
    let t = Instant::now();
    for _ in 0..200_000 {
        one(&mut rng, &mut lat, &s)?;
    }
    rep.num("get_1t_kops", 200.0 / t.elapsed().as_secs_f64());
    lat.report(rep, "get_1t");
    drop(s);

    let engine = db.get();
    let t = Instant::now();
    let lats: Vec<Lat> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|i| {
                scope.spawn(move || {
                    let mut rng = Rng::new(ctx.seed + 100 + i);
                    let s = engine.snapshot().unwrap();
                    let mut lat = Lat::new();
                    for _ in 0..100_000 {
                        one(&mut rng, &mut lat, &s).unwrap();
                    }
                    lat
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    rep.num("get_4t_kops", 400.0 / t.elapsed().as_secs_f64());
    let mut all = Lat::new();
    for l in &lats {
        all.merge(l);
    }
    all.report(rep, "get_4t");
    Ok(())
}

/// Scans of 100 consecutive records from random start keys.
fn range_scans<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let db = preload_and_reopen::<E>(ctx, &mut rng, rep)?;
    let s = db.get().snapshot()?;
    let mut lat = Lat::new();
    let t = Instant::now();
    let mut rows = 0;
    for _ in 0..20_000 {
        let start = format!("user{:016x}", rng.next_u64()).into_bytes();
        rows += lat.time(|| s.scan(Bound::Included(&start), Bound::Unbounded, 100))?.len();
    }
    let secs = t.elapsed().as_secs_f64();
    rep.num("scan100_kops", 20.0 / secs);
    rep.num("scan_rows_per_s_k", rows as f64 / secs / 1000.0);
    lat.report(rep, "scan100");
    Ok(())
}

/// Single-record overwrite commits: the transactional latency profile.
fn small_commits<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let db = preload_and_reopen::<E>(ctx, &mut rng, rep)?;
    let commits = if ctx.opts.durability == Durability::Sync { 10_000 } else { 50_000 };
    let io0 = db.io.snapshot();
    let (_, w0) = proc_io();
    let mut lat = Lat::new();
    let mut logical = 0;
    let t = Instant::now();
    for _ in 0..commits {
        let mut b = WriteBatch::new();
        b.put(user_key(rng.below(ctx.n)), rng.bytes(200));
        logical += b.payload_bytes() as u64;
        lat.time(|| db.get().commit(&b))?;
    }
    let secs = t.elapsed().as_secs_f64();
    let (_, w1) = proc_io();
    rep.num("commits_per_s", commits as f64 / secs);
    lat.report(rep, "commit");
    rep.num("write_amp", (w1 - w0) as f64 / logical as f64);
    io_report(rep, "commit", &db.io.snapshot().since(&io0), commits);
    Ok(())
}

/// Append-heavy: monotonically increasing keys (event-log / UUIDv7 order)
/// inserted in commits of 100, then two full passes of in-place updates.
fn append_seq<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let key = |i: u64| format!("log/{i:016}").into_bytes();
    let db = Db::<E>::open(ctx)?;
    let mut logical = 0u64;
    let (_, w0) = proc_io();
    let mut lat = Lat::new();
    let t = Instant::now();
    let mut b = WriteBatch::new();
    for i in 0..ctx.n {
        b.put(key(i), val(&mut rng, 100, 400));
        if b.len() == 100 || i + 1 == ctx.n {
            logical += b.payload_bytes() as u64;
            lat.time(|| db.get().commit(&b))?;
            b = WriteBatch::new();
        }
    }
    rep.num("append_kops", ctx.n as f64 / t.elapsed().as_secs_f64() / 1000.0);
    lat.report(rep, "append_commit100");
    let mut lat = Lat::new();
    let t = Instant::now();
    for _pass in 0..2 {
        for i in 0..ctx.n {
            b.put(key(i), val(&mut rng, 100, 400));
            if b.len() == 100 || i + 1 == ctx.n {
                logical += b.payload_bytes() as u64;
                lat.time(|| db.get().commit(&b))?;
                b = WriteBatch::new();
            }
        }
    }
    rep.num("update_kops", 2.0 * ctx.n as f64 / t.elapsed().as_secs_f64() / 1000.0);
    lat.report(rep, "update_commit100");
    let (_, w1) = proc_io();
    rep.num("write_amp", (w1 - w0) as f64 / logical as f64);
    // Expected live bytes: key (20) + mean value (250) per record.
    let live = ctx.n * (20 + 250);
    rep.num("space_amp", dir_size(&ctx.dir) as f64 / live as f64);
    Ok(())
}

/// Read-dominated mix: 4 threads, each doing 90% point reads and 10%
/// single-record durable updates (the writer is serialized by the engine).
fn mixed<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let db = preload_and_reopen::<E>(ctx, &mut rng, rep)?;
    let engine = db.get();
    let n = ctx.n;
    let ops_per_thread = 20_000;
    let t = Instant::now();
    let results: Vec<(Lat, Lat)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4u64)
            .map(|i| {
                scope.spawn(move || {
                    let mut rng = Rng::new(ctx.seed + 1000 + i);
                    let (mut reads, mut writes) = (Lat::new(), Lat::new());
                    for _ in 0..ops_per_thread {
                        let k = user_key(rng.below(n));
                        if rng.chance(1, 10) {
                            let mut b = WriteBatch::new();
                            b.put(k, rng.bytes(200));
                            writes.time(|| engine.commit(&b)).unwrap();
                        } else {
                            let got = reads.time(|| engine.snapshot().and_then(|s| s.get(&k))).unwrap();
                            assert!(got.is_some());
                        }
                    }
                    (reads, writes)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let secs = t.elapsed().as_secs_f64();
    rep.num("mixed_kops", 4.0 * ops_per_thread as f64 / secs / 1000.0);
    let (mut reads, mut writes) = (Lat::new(), Lat::new());
    for (r, w) in &results {
        reads.merge(r);
        writes.merge(w);
    }
    rep.num("write_fraction", writes.count() as f64 / (reads.count() + writes.count()) as f64);
    reads.report(rep, "read");
    writes.report(rep, "write");
    Ok(())
}

/// Simulated revision history (SAD §17.3 item 6): every update writes the
/// current value plus a revision entry, using the "snapshot every 8
/// revisions, structural delta otherwise" layout from the revision-model
/// design note. Measures the write cost of keeping history, the space it
/// takes, and the cost of reading the history / reconstructing an old
/// revision.
fn history_sim<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    const REVS: u64 = 16;
    const SNAPSHOT_EVERY: u64 = 8;
    let mut rng = Rng::new(ctx.seed);
    let records = (ctx.n / REVS).max(100);
    let cur = |id: u64| format!("cur/{:016x}", mix(id)).into_bytes();
    let rev = |id: u64, r: u64| format!("rev/{:016x}/{r:08}", mix(id)).into_bytes();
    let db = Db::<E>::open(ctx)?;
    let mut lat = Lat::new();
    let mut current_bytes = 0u64;
    let mut history_bytes = 0u64;
    let t = Instant::now();
    // Revision r of every record, round-robin, so revisions of one record
    // are spread over time like real edits.
    for r in 0..REVS {
        let mut b = WriteBatch::new();
        for id in 0..records {
            let doc = rng.bytes(500);
            let entry = if r % SNAPSHOT_EVERY == 0 { doc.clone() } else { rng.bytes(60) };
            history_bytes += entry.len() as u64;
            if r == REVS - 1 {
                current_bytes += doc.len() as u64;
            }
            b.put(cur(id), doc).put(rev(id, r), entry);
            if b.len() >= 200 {
                lat.time(|| db.get().commit(&b))?;
                b = WriteBatch::new();
            }
        }
        lat.time(|| db.get().commit(&b))?;
    }
    rep.num("revisions_per_s_k", (records * REVS) as f64 / t.elapsed().as_secs_f64() / 1000.0);
    lat.report(rep, "rev_commit100");
    rep.num("history_overhead", (current_bytes + history_bytes) as f64 / current_bytes as f64);
    rep.num("space_amp", dir_size(&ctx.dir) as f64 / (current_bytes + history_bytes) as f64);
    let s = db.get().snapshot()?;
    let (mut hist, mut old) = (Lat::new(), Lat::new());
    for _ in 0..5_000 {
        let id = rng.below(records);
        let lo = format!("rev/{:016x}/", mix(id)).into_bytes();
        let hi = format!("rev/{:016x}0", mix(id)).into_bytes();
        let rows = hist.time(|| s.scan(Bound::Included(&lo), Bound::Excluded(&hi), 100))?;
        assert_eq!(rows.len() as u64, REVS);
        // Reconstruct revision r: nearest snapshot at or below r + deltas.
        let r = rng.below(REVS);
        let base = r - r % SNAPSHOT_EVERY;
        let rows = old.time(|| s.scan(Bound::Included(&rev(id, base)), Bound::Included(&rev(id, r)), 100))?;
        assert_eq!(rows.len() as u64, r - base + 1);
    }
    hist.report(rep, "history_scan");
    old.report(rep, "reconstruct");
    Ok(())
}

/// LiveSpec-like: code symbols grouped by module, updated in change sets
/// (one commit per change set + a change-set record), with symbol lookups
/// and per-module listings between change sets.
fn livespec<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let modules = (ctx.n / 200).max(10);
    let sym = |m: u64, s: u64| format!("sym/{m:06}/{s:05}").into_bytes();
    let db = Db::<E>::open(ctx)?;
    let mut b = WriteBatch::new();
    for m in 0..modules {
        for s in 0..200 {
            b.put(sym(m, s), val(&mut rng, 200, 800));
            if b.len() == 1000 {
                db.get().commit(&b)?;
                b = WriteBatch::new();
            }
        }
    }
    db.get().commit(&b)?;
    let io0 = db.io.snapshot();
    let (mut commit, mut get, mut scan) = (Lat::new(), Lat::new(), Lat::new());
    let steps = 2000;
    let t = Instant::now();
    for cs in 0..steps {
        let m = rng.below(modules);
        let mut b = WriteBatch::new();
        for _ in 0..rng.range(20, 50) {
            b.put(sym(m, rng.below(200)), val(&mut rng, 200, 800));
        }
        b.put(format!("cs/{cs:012}").into_bytes(), rng.bytes(300));
        commit.time(|| db.get().commit(&b))?;
        let s = db.get().snapshot()?;
        for _ in 0..20 {
            let k = sym(rng.below(modules), rng.below(200));
            assert!(get.time(|| s.get(&k))?.is_some());
        }
        for _ in 0..2 {
            let m = rng.below(modules);
            let lo = format!("sym/{m:06}/").into_bytes();
            let hi = format!("sym/{m:06}0").into_bytes();
            let rows = scan.time(|| s.scan(Bound::Included(&lo), Bound::Excluded(&hi), 500))?;
            assert_eq!(rows.len(), 200);
        }
    }
    rep.num("changesets_per_s", steps as f64 / t.elapsed().as_secs_f64());
    commit.report(rep, "changeset_commit");
    get.report(rep, "symbol_get");
    scan.report(rep, "module_scan");
    io_report(rep, "changeset", &db.io.snapshot().since(&io0), steps);
    Ok(())
}

/// Cauce-like: task state machine. Each step durably updates a task's status
/// and appends an event, then reads the status and a page of events. Ends
/// with an unclean shutdown and measures recovery.
fn cauce<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let tasks = (ctx.n / 20).max(100);
    let status = |t: u64| format!("task/{t:08}").into_bytes();
    let event = |t: u64, e: u64| format!("evt/{t:08}/{e:08}").into_bytes();
    let mut next_event = vec![0u64; tasks as usize];
    let mut db = Db::<E>::open(ctx)?;
    let mut b = WriteBatch::new();
    for i in 0..ctx.n {
        let t = i % tasks;
        b.put(event(t, next_event[t as usize]), rng.bytes(300));
        next_event[t as usize] += 1;
        if i < tasks {
            b.put(status(t), rng.bytes(150));
        }
        if b.len() >= 1000 {
            db.get().commit(&b)?;
            b = WriteBatch::new();
        }
    }
    db.get().commit(&b)?;
    let io0 = db.io.snapshot();
    let (mut commit, mut read) = (Lat::new(), Lat::new());
    let steps = 10_000;
    let t = Instant::now();
    let mut last = 0;
    for _ in 0..steps {
        let task = rng.below(tasks);
        let mut b = WriteBatch::new();
        b.put(status(task), rng.bytes(150));
        b.put(event(task, next_event[task as usize]), rng.bytes(300));
        next_event[task as usize] += 1;
        commit.time(|| db.get().commit(&b))?;
        read.time(|| -> Result<()> {
            let s = db.get().snapshot()?;
            assert!(s.get(&status(task))?.is_some());
            let lo = format!("evt/{task:08}/").into_bytes();
            let hi = format!("evt/{task:08}0").into_bytes();
            assert!(!s.scan(Bound::Included(&lo), Bound::Excluded(&hi), 10)?.is_empty());
            Ok(())
        })?;
        last = task;
    }
    rep.num("steps_per_s", steps as f64 / t.elapsed().as_secs_f64());
    commit.report(rep, "step_commit");
    read.report(rep, "step_read");
    io_report(rep, "step", &db.io.snapshot().since(&io0), steps);
    // Unclean shutdown (no close), then recovery.
    drop(db.db.take());
    let t = Instant::now();
    db.reopen(ctx)?;
    assert!(db.get().snapshot()?.get(&status(last))?.is_some());
    rep.num("recovery_ms", t.elapsed().as_secs_f64() * 1000.0);
    db.close()
}

/// Agent memory: time-ordered memories (1-4 KiB) with a 768-float embedding
/// each, inserted one per commit; random recall, "recent" range scans and
/// periodic expiry of the oldest memories.
fn agent_memory<E: Engine>(ctx: &Ctx, rep: &mut Report) -> Result<()> {
    let mut rng = Rng::new(ctx.seed);
    let mut clock = 1_700_000_000_000u64;
    let mut ids: VecDeque<String> = VecDeque::new();
    let mut new_id = |rng: &mut Rng| {
        clock += rng.range(1, 50);
        format!("{clock:012x}{:020x}", rng.next_u64() as u128 * 0x1_0000 + rng.below(0x1_0000) as u128)
    };
    let mem = |id: &str| format!("mem/{id}").into_bytes();
    let emb = |id: &str| format!("emb/{id}").into_bytes();
    let db = Db::<E>::open(ctx)?;
    let initial = (ctx.n / 4).max(100);
    let mut live_bytes: i64 = 0;
    let mut b = WriteBatch::new();
    for _ in 0..initial {
        let id = new_id(&mut rng);
        let m = val(&mut rng, 1024, 4096);
        live_bytes += (m.len() + 3072 + 2 * 36) as i64;
        b.put(mem(&id), m).put(emb(&id), rng.bytes(3072));
        ids.push_back(id);
        if b.len() >= 200 {
            db.get().commit(&b)?;
            b = WriteBatch::new();
        }
    }
    db.get().commit(&b)?;
    let io0 = db.io.snapshot();
    let (mut write, mut recall, mut recent, mut expire) = (Lat::new(), Lat::new(), Lat::new(), Lat::new());
    let steps = 5_000;
    let t = Instant::now();
    for step in 0..steps {
        let id = new_id(&mut rng);
        let m = val(&mut rng, 1024, 4096);
        live_bytes += (m.len() + 3072 + 2 * 36) as i64;
        let mut b = WriteBatch::new();
        b.put(mem(&id), m).put(emb(&id), rng.bytes(3072));
        write.time(|| db.get().commit(&b))?;
        ids.push_back(id);
        let s = db.get().snapshot()?;
        for _ in 0..10 {
            let id = &ids[rng.below(ids.len() as u64) as usize];
            recall.time(|| -> Result<()> {
                assert!(s.get(&mem(id))?.is_some());
                assert!(s.get(&emb(id))?.is_some());
                Ok(())
            })?;
        }
        let from = mem(&ids[ids.len().saturating_sub(50)]);
        let rows = recent.time(|| s.scan(Bound::Included(&from), Bound::Excluded(b"mem0"), 20))?;
        assert_eq!(rows.len(), 20);
        drop(s);
        if step % 100 == 99 {
            let mut b = WriteBatch::new();
            for _ in 0..ids.len() / 50 {
                let id = ids.pop_front().unwrap();
                b.delete(mem(&id)).delete(emb(&id));
                live_bytes -= 2 * 3072; // approximation of the removed payload
            }
            expire.time(|| db.get().commit(&b))?;
        }
    }
    rep.num("steps_per_s", steps as f64 / t.elapsed().as_secs_f64());
    write.report(rep, "memory_write");
    recall.report(rep, "recall");
    recent.report(rep, "recent_scan");
    expire.report(rep, "expire_commit");
    rep.num("space_amp", dir_size(&ctx.dir) as f64 / live_bytes.max(1) as f64);
    io_report(rep, "memory_write", &db.io.snapshot().since(&io0), steps);
    Ok(())
}
