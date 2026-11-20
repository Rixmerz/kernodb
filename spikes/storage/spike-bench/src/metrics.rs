use std::path::Path;
use std::time::Instant;

use hdrhistogram::Histogram;
use serde_json::{Map, Value};

/// Latency recorder (nanoseconds).
pub struct Lat {
    h: Histogram<u64>,
}

impl Default for Lat {
    fn default() -> Self {
        Self::new()
    }
}

impl Lat {
    pub fn new() -> Self {
        Lat { h: Histogram::new_with_bounds(1, 60_000_000_000, 3).unwrap() }
    }

    pub fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.record_nanos(t.elapsed().as_nanos() as u64);
        out
    }

    /// Records a duration measured by the caller, like [`Lat::time`].
    pub fn record_nanos(&mut self, nanos: u64) {
        self.h.saturating_record(nanos.max(1));
    }

    pub fn merge(&mut self, other: &Lat) {
        self.h.add(&other.h).unwrap();
    }

    pub fn count(&self) -> u64 {
        self.h.len()
    }

    /// Adds `<name>_p50_us`, `_p95_us`, `_p99_us`, `_p999_us` and `_max_us`.
    pub fn report(&self, rep: &mut Report, name: &str) {
        let us = |q: f64| self.h.value_at_quantile(q) as f64 / 1000.0;
        rep.num(&format!("{name}_p50_us"), us(0.50));
        rep.num(&format!("{name}_p95_us"), us(0.95));
        rep.num(&format!("{name}_p99_us"), us(0.99));
        rep.num(&format!("{name}_p999_us"), us(0.999));
        rep.num(&format!("{name}_max_us"), self.h.max() as f64 / 1000.0);
    }
}

/// Flat JSON object of results.
#[derive(Default)]
pub struct Report {
    pub fields: Map<String, Value>,
}

impl Report {
    pub fn str(&mut self, k: &str, v: &str) {
        self.fields.insert(k.into(), Value::String(v.into()));
    }

    pub fn num(&mut self, k: &str, v: f64) {
        let v = if v.is_finite() { (v * 1000.0).round() / 1000.0 } else { 0.0 };
        self.fields.insert(k.into(), serde_json::json!(v));
    }

    pub fn to_json(&self) -> String {
        Value::Object(self.fields.clone()).to_string()
    }
}

/// `(rchar, wchar)` for this process: bytes passed to read/write syscalls
/// (including `pread`/`pwrite`). Used as a uniform write-volume measure for
/// every engine, including the ones that do not go through our VFS.
pub fn proc_io() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/self/io").unwrap_or_default();
    let field = |name: &str| {
        s.lines().find_map(|l| l.strip_prefix(name).and_then(|v| v.trim().parse::<u64>().ok())).unwrap_or(0)
    };
    (field("rchar:"), field("wchar:"))
}

/// The 1, 5 and 15 minute load averages of the machine (`uptime`), or
/// zeros where `/proc/loadavg` is missing. Benchmarks record them because
/// other work on the machine changes absolute latencies.
pub fn load_average() -> [f64; 3] {
    let s = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut it = s.split_whitespace().map(|v| v.parse().unwrap_or(0.0));
    [it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0)]
}

/// Peak resident set size of this process in KiB.
pub fn peak_rss_kib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find_map(|l| l.strip_prefix("VmHWM:").and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok()))
        .unwrap_or(0)
}

/// Total size of the regular files under `dir` (recursive).
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                total += dir_size(&e.path());
            } else {
                total += md.len();
            }
        }
    }
    total
}
