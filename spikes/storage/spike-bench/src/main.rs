//! Benchmark driver for the Kerno Phase 0 storage spike (M0).
//!
//! ```text
//! spike-bench suite  [--engines btree,lsm,redb,fjall,sqlite] [--workloads all|a,b]
//!                    [--scales small,medium] [--reps 3] [--durability sync|buffered]
//!                    [--cold true] [--dir bench-data] [--out results.jsonl]
//! spike-bench run    --engine E --workload W --n N [--durability sync] [--dir D] [--seed S]
//! spike-bench report --input results.jsonl [--env env.json]   (Markdown to stdout)
//! spike-bench env    [--dir bench-data]                        (JSON to stdout)
//! ```
//!
//! `suite` runs every (engine, workload, scale) combination `reps` times,
//! each in a fresh child process (clean peak-RSS and I/O counters) and a
//! fresh directory, interleaving engines within each repetition so that
//! slow drifts of the machine do not favour one engine.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::Value;
use spike_core::{Durability, Engine, Options};

use spike_bench::adapters;
use spike_bench::metrics::{self, Report, peak_rss_kib};
use spike_bench::workloads::{self, Ctx, WORKLOADS};

const ENGINES: &[&str] = &["btree", "lsm", "redb", "fjall", "sqlite"];

fn scale_n(scale: &str) -> u64 {
    match scale {
        "tiny" => 5_000,
        "small" => 20_000,
        "medium" => 200_000,
        "large" => 1_000_000,
        other => other.parse().unwrap_or_else(|_| panic!("unknown scale {other}")),
    }
}

struct Args(Vec<String>);

impl Args {
    fn get(&self, name: &str) -> Option<String> {
        let flag = format!("--{name}");
        self.0.iter().position(|a| *a == flag).and_then(|i| self.0.get(i + 1).cloned())
    }

    fn get_or(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or_else(|| default.to_string())
    }

    fn list(&self, name: &str, all: &[&str], default: &str) -> Vec<String> {
        let v = self.get_or(name, default);
        if v == "all" { all.iter().map(|s| s.to_string()).collect() } else { v.split(',').map(String::from).collect() }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprintln!("usage: spike-bench <suite|run|report|env> [options] (see source docs)");
        std::process::exit(2);
    };
    let args = Args(argv[1..].to_vec());
    match cmd.as_str() {
        "run" => cmd_run(&args),
        "suite" => cmd_suite(&args),
        "report" => cmd_report(&args),
        "env" => println!("{}", env_info(Path::new(&args.get_or("dir", "bench-data")))),
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    }
}

fn run_engine<E: Engine>(workload: &str, ctx: &Ctx, rep: &mut Report) {
    if let Err(e) = workloads::run::<E>(workload, ctx, rep) {
        panic!("{} / {workload}: {e}", E::name());
    }
}

fn dispatch(engine: &str, workload: &str, ctx: &Ctx, rep: &mut Report) {
    match engine {
        #[cfg(feature = "candidates")]
        "btree" => run_engine::<spike_btree::BTreeEngine>(workload, ctx, rep),
        #[cfg(feature = "candidates")]
        "lsm" => run_engine::<spike_lsm::LsmEngine>(workload, ctx, rep),
        "redb" => run_engine::<adapters::RedbEngine>(workload, ctx, rep),
        "fjall" => run_engine::<adapters::FjallEngine>(workload, ctx, rep),
        "sqlite" => run_engine::<adapters::SqliteEngine>(workload, ctx, rep),
        "log" => run_engine::<spike_core::reference::LogEngine>(workload, ctx, rep),
        other => panic!("unknown or disabled engine {other}"),
    }
}

fn cmd_run(args: &Args) {
    let engine = args.get("engine").expect("--engine");
    let workload = args.get("workload").expect("--workload");
    let n = scale_n(&args.get_or("n", "small"));
    let durability = match args.get_or("durability", "sync").as_str() {
        "sync" => Durability::Sync,
        "buffered" => Durability::Buffered,
        d => panic!("unknown durability {d}"),
    };
    let dir = PathBuf::from(args.get_or("dir", "bench-data")).join(format!("{engine}-{workload}-{n}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    let opts = Options { durability, cache_bytes: 64 << 20, write_buffer_bytes: 8 << 20 };
    let cold = args.get_or("cold", "false") == "true";
    let ctx = Ctx { dir: dir.clone(), opts, n, seed: args.get_or("seed", "42").parse().unwrap(), cold };
    let mut rep = Report::default();
    rep.str("engine", &engine);
    rep.str("workload", &workload);
    rep.num("n", n as f64);
    rep.str("durability", &args.get_or("durability", "sync"));
    rep.str("cache", if cold { "cold" } else { "warm" });
    let t = Instant::now();
    dispatch(&engine, &workload, &ctx, &mut rep);
    rep.num("wall_s", t.elapsed().as_secs_f64());
    rep.num("peak_rss_mb", peak_rss_kib() as f64 / 1024.0);
    let _ = std::fs::remove_dir_all(&dir);
    println!("{}", rep.to_json());
}

fn cmd_suite(args: &Args) {
    let engines = args.list("engines", ENGINES, &ENGINES.join(","));
    let workloads = args.list("workloads", WORKLOADS, "all");
    let scales = args.list("scales", &[], "small");
    let reps: u32 = args.get_or("reps", "3").parse().unwrap();
    let durability = args.get_or("durability", "sync");
    let cold = args.get_or("cold", "false");
    let dir = args.get_or("dir", "bench-data");
    let out = PathBuf::from(args.get_or("out", "results.jsonl"));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&out).unwrap();
    let total = scales.len() * workloads.len() * reps as usize * engines.len();
    let mut done = 0;
    for scale in &scales {
        for workload in &workloads {
            for rep in 0..reps {
                for engine in &engines {
                    done += 1;
                    eprint!("[{done}/{total}] {scale} {workload} {engine} rep {rep} ... ");
                    let t = Instant::now();
                    let output = Command::new(&exe)
                        .args(["run", "--engine", engine, "--workload", workload, "--n", scale])
                        .args(["--durability", &durability, "--dir", &dir, "--seed", &(42 + rep).to_string()])
                        .args(["--cold", &cold])
                        .output()
                        .unwrap();
                    let line = String::from_utf8_lossy(&output.stdout).lines().last().unwrap_or("").to_string();
                    if !output.status.success() || line.is_empty() {
                        eprintln!("FAILED\n{}", String::from_utf8_lossy(&output.stderr));
                        let mut v: serde_json::Map<String, Value> = serde_json::Map::new();
                        v.insert("engine".into(), engine.clone().into());
                        v.insert("workload".into(), workload.clone().into());
                        v.insert("n".into(), scale_n(scale).into());
                        v.insert("durability".into(), durability.clone().into());
                        v.insert("error".into(), String::from_utf8_lossy(&output.stderr).into_owned().into());
                        writeln!(file, "{}", Value::Object(v)).unwrap();
                        continue;
                    }
                    let mut v: Value = serde_json::from_str(&line).unwrap();
                    v["rep"] = rep.into();
                    writeln!(file, "{v}").unwrap();
                    eprintln!("{:.1}s", t.elapsed().as_secs_f64());
                }
            }
        }
    }
}

/// Headline metrics per workload, in table column order.
fn headline(workload: &str) -> &'static [&'static str] {
    match workload {
        "bulk_load" => {
            &["load_kops", "write_amp", "space_amp_closed", "close_ms", "reopen_ms", "recovery_ms", "peak_rss_mb"]
        }
        "point_reads" => {
            &["get_1t_kops", "get_1t_p50_us", "get_1t_p95_us", "get_1t_p99_us", "get_4t_kops", "get_4t_p99_us"]
        }
        "range_scans" => &["scan100_kops", "scan100_p50_us", "scan100_p95_us", "scan100_p99_us"],
        "small_commits" => &[
            "commits_per_s",
            "commit_p50_us",
            "commit_p95_us",
            "commit_p99_us",
            "commit_p999_us",
            "write_amp",
            "commit_fsyncs_per_commit",
        ],
        "point_deletes" => &[
            "delete_commits_per_s",
            "delete_commit_p50_us",
            "delete_commit_p99_us",
            "get_deleted_p50_us",
            "get_deleted_p99_us",
            "get_live_p99_us",
        ],
        "append_seq" => {
            &["append_kops", "update_kops", "update_commit100_p99_us", "write_amp", "space_amp", "peak_rss_mb"]
        }
        "mixed" => &["mixed_kops", "read_p50_us", "read_p99_us", "write_p50_us", "write_p99_us"],
        "history_sim" => &[
            "revisions_per_s_k",
            "rev_commit100_p99_us",
            "space_amp",
            "history_scan_p50_us",
            "history_scan_p99_us",
            "reconstruct_p99_us",
        ],
        "livespec" => &[
            "changesets_per_s",
            "changeset_commit_p50_us",
            "changeset_commit_p99_us",
            "symbol_get_p99_us",
            "module_scan_p50_us",
            "module_scan_p99_us",
        ],
        "cauce" => &["steps_per_s", "step_commit_p50_us", "step_commit_p99_us", "step_read_p99_us", "recovery_ms"],
        "agent_memory" => &[
            "steps_per_s",
            "memory_write_p50_us",
            "memory_write_p99_us",
            "recall_p99_us",
            "recent_scan_p99_us",
            "expire_commit_p99_us",
            "space_amp",
        ],
        _ => &[],
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn fmt_num(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn cmd_report(args: &Args) {
    let input = args.get_or("input", "results.jsonl");
    let text = std::fs::read_to_string(&input).unwrap();
    // (workload, durability, n) -> engine -> metric -> values
    type Cell = BTreeMap<String, Vec<f64>>;
    let mut groups: BTreeMap<(usize, String, u64), BTreeMap<String, Cell>> = BTreeMap::new();
    let mut failures = Vec::new();
    let engine_order = |e: &str| ENGINES.iter().position(|x| *x == e).unwrap_or(99);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let workload = v["workload"].as_str().unwrap().to_string();
        let engine = v["engine"].as_str().unwrap().to_string();
        let n = v["n"].as_f64().unwrap() as u64;
        let mut durability = v["durability"].as_str().unwrap_or("sync").to_string();
        if v["cache"].as_str() == Some("cold") {
            // Only call it cold if dropping the OS cache actually worked.
            durability += if v["cold_cache"].as_f64() == Some(1.0) {
                ", cold cache"
            } else {
                ", cold cache requested but NOT applied (needs root)"
            };
        }
        if let Some(e) = v.get("error") {
            failures.push(format!(
                "{engine} / {workload} / n={n}: {}",
                e.as_str().unwrap_or("").lines().last().unwrap_or("")
            ));
            continue;
        }
        let w_idx = WORKLOADS.iter().position(|w| *w == workload).unwrap_or(99);
        let cell = groups.entry((w_idx, durability, n)).or_default().entry(engine).or_default();
        for (k, val) in v.as_object().unwrap() {
            if let Some(x) = val.as_f64() {
                cell.entry(k.clone()).or_default().push(x);
            }
        }
    }
    let mut out = String::new();
    for ((w_idx, durability, n), engines) in &groups {
        let workload = WORKLOADS.get(*w_idx).copied().unwrap_or("?");
        let cols = headline(workload);
        out += &format!("\n#### `{workload}` — n = {n}, durability = {durability}\n\n");
        out += &format!("| engine | reps | {} |\n", cols.join(" | "));
        out += &format!("|---|---|{}\n", "---:|".repeat(cols.len()));
        let mut names: Vec<&String> = engines.keys().collect();
        names.sort_by_key(|e| engine_order(e));
        for e in names {
            let cell = &engines[e];
            let reps = cell.get("wall_s").map(|v| v.len()).unwrap_or(0);
            let vals: Vec<String> = cols
                .iter()
                .map(|c| match cell.get(*c) {
                    Some(v) if !v.is_empty() => {
                        let med = median(v.clone());
                        let lo = v.iter().cloned().fold(f64::INFINITY, f64::min);
                        let hi = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                        if v.len() > 1 && (hi - lo) > 0.15 * med.abs() {
                            format!("{} ({}–{})", fmt_num(med), fmt_num(lo), fmt_num(hi))
                        } else {
                            fmt_num(med)
                        }
                    }
                    _ => "—".into(),
                })
                .collect();
            out += &format!("| {e} | {reps} | {} |\n", vals.join(" | "));
        }
    }
    if !failures.is_empty() {
        out += "\n#### Failed runs\n\n";
        for f in failures {
            out += &format!("- {f}\n");
        }
    }
    println!("{out}");
}

fn env_info(dir: &Path) -> String {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let cpu = read("/proc/cpuinfo")
        .lines()
        .find_map(|l| l.strip_prefix("model name").map(|v| v.trim_start_matches([' ', '\t', ':']).to_string()))
        .unwrap_or_default();
    let mem_kib: u64 = read("/proc/meminfo")
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:").and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok()))
        .unwrap_or(0);
    let cmd = |c: &str, a: &[&str]| {
        Command::new(c)
            .args(a)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    // fdatasync latency of the benchmark file system: the floor for any
    // durable commit, and the number that most changes between machines.
    std::fs::create_dir_all(dir).unwrap();
    let probe = dir.join("fsync-probe");
    let f = std::fs::File::create(&probe).unwrap();
    let mut lat = metrics::Lat::new();
    for i in 0..500u64 {
        use std::os::unix::fs::FileExt;
        f.write_all_at(&[i as u8; 4096], (i % 64) * 4096).unwrap();
        lat.time(|| f.sync_data().unwrap());
    }
    drop(f);
    let _ = std::fs::remove_file(&probe);
    let mut rep = Report::default();
    lat.report(&mut rep, "fdatasync_4k");
    let mut v = rep.fields;
    v.insert("cpu".into(), cpu.into());
    v.insert("cores".into(), std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0).into());
    v.insert("mem_gib".into(), ((mem_kib as f64 / 1024.0 / 1024.0 * 10.0).round() / 10.0).into());
    v.insert("kernel".into(), read("/proc/sys/kernel/osrelease").trim().into());
    v.insert("rustc".into(), cmd("rustc", &["--version"]).into());
    v.insert("git_commit".into(), cmd("git", &["rev-parse", "--short", "HEAD"]).into());
    let fs = cmd("df", &["-T", dir.to_str().unwrap()])
        .lines()
        .nth(1)
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    v.insert("filesystem".into(), fs.into());
    Value::Object(v).to_string()
}
