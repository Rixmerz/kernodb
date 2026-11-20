//! The counters `GET /metrics` serves, in the Prometheus text format (version 0.0.4).
//!
//! No label names a tenant, a token or a path: the endpoint needs no token, so it must not tell
//! anyone who uses the server.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Mutex;
use std::time::Duration;

/// The upper bounds, in seconds, of the request-duration histogram buckets.
const BUCKETS: [f64; 10] = [0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 1.0, 5.0];

#[derive(Default)]
struct Histogram {
    /// Requests per bucket, not cumulative; the last slot is over the last bound.
    counts: [u64; BUCKETS.len() + 1],
    sum: f64,
}

#[derive(Default)]
struct Inner {
    http: BTreeMap<u16, u64>,
    grpc: BTreeMap<&'static str, u64>,
    auth_failures: u64,
    durations: BTreeMap<&'static str, Histogram>,
}

/// What the backups of every database did since the server started, summed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackupTotals {
    pub commits: u64,
    pub segments: u64,
    pub bases: u64,
    pub errors: u64,
}

/// ponytail: one mutex for every counter, held for a few nanoseconds per request; split it into
/// atomics if it ever shows in a profile.
#[derive(Default)]
pub struct Metrics(Mutex<Inner>);

impl Metrics {
    fn with(&self, f: impl FnOnce(&mut Inner)) {
        // A poisoned lock still holds valid counters: a panic cannot leave a count half-written.
        f(&mut self.0.lock().unwrap_or_else(|p| p.into_inner()));
    }

    fn time(inner: &mut Inner, transport: &'static str, took: Duration) {
        let h = inner.durations.entry(transport).or_default();
        let secs = took.as_secs_f64();
        let slot = BUCKETS.iter().position(|&le| secs <= le).unwrap_or(BUCKETS.len());
        h.counts[slot] += 1;
        h.sum += secs;
    }

    /// One HTTP answer with this status, `took` after the request arrived.
    pub fn http(&self, status: u16, took: Duration) {
        self.with(|m| {
            *m.http.entry(status).or_default() += 1;
            Self::time(m, "http", took);
        });
    }

    /// One gRPC call ended with this error `code` (`"ok"` when it succeeded).
    pub fn grpc(&self, code: &'static str, took: Duration) {
        self.with(|m| {
            *m.grpc.entry(code).or_default() += 1;
            Self::time(m, "grpc", took);
        });
    }

    /// A request refused for a missing or unknown token, on either transport.
    pub fn auth_failure(&self) {
        self.with(|m| m.auth_failures += 1);
    }

    /// The text of `GET /metrics`. `databases` is how many databases the server holds open, and
    /// `backup` what their backups did (`None` when backups are off).
    pub fn render(&self, databases: usize, backup: Option<BackupTotals>) -> String {
        let mut out = String::new();
        self.with(|m| {
            let _ = writeln!(
                out,
                "# HELP kerno_http_responses_total HTTP answers by status.\n\
                 # TYPE kerno_http_responses_total counter"
            );
            for (status, n) in &m.http {
                let _ = writeln!(out, "kerno_http_responses_total{{status=\"{status}\"}} {n}");
            }
            let _ = writeln!(
                out,
                "# HELP kerno_grpc_calls_total gRPC calls by error code, ok when they succeeded.\n\
                 # TYPE kerno_grpc_calls_total counter"
            );
            for (code, n) in &m.grpc {
                let _ = writeln!(out, "kerno_grpc_calls_total{{code=\"{code}\"}} {n}");
            }
            let _ = writeln!(
                out,
                "# HELP kerno_auth_failures_total Requests refused for a missing or unknown token.\n\
                 # TYPE kerno_auth_failures_total counter\nkerno_auth_failures_total {}",
                m.auth_failures
            );
            let _ = writeln!(
                out,
                "# HELP kerno_request_duration_seconds Time to answer a request, by transport.\n\
                 # TYPE kerno_request_duration_seconds histogram"
            );
            for (transport, h) in &m.durations {
                let mut cumulative = 0;
                for (le, n) in BUCKETS.iter().map(ToString::to_string).chain(["+Inf".to_owned()]).zip(h.counts) {
                    cumulative += n;
                    let _ = writeln!(
                        out,
                        "kerno_request_duration_seconds_bucket{{transport=\"{transport}\",le=\"{le}\"}} {cumulative}"
                    );
                }
                let _ = writeln!(out, "kerno_request_duration_seconds_sum{{transport=\"{transport}\"}} {}", h.sum);
                let _ = writeln!(out, "kerno_request_duration_seconds_count{{transport=\"{transport}\"}} {cumulative}");
            }
        });
        if let Some(b) = backup {
            let _ = writeln!(
                out,
                "# HELP kerno_backup_shipped_commits_total Commits shipped to the backup store.\n\
                 # TYPE kerno_backup_shipped_commits_total counter\nkerno_backup_shipped_commits_total {}\n\
                 # HELP kerno_backup_segments_total Write-ahead log segments uploaded.\n\
                 # TYPE kerno_backup_segments_total counter\nkerno_backup_segments_total {}\n\
                 # HELP kerno_backup_bases_total Base backups taken.\n\
                 # TYPE kerno_backup_bases_total counter\nkerno_backup_bases_total {}\n\
                 # HELP kerno_backup_errors_total Shipments and base backups that failed.\n\
                 # TYPE kerno_backup_errors_total counter\nkerno_backup_errors_total {}",
                b.commits, b.segments, b.bases, b.errors
            );
        }
        let _ = writeln!(
            out,
            "# HELP kerno_databases_open Databases the server holds open.\n\
             # TYPE kerno_databases_open gauge\nkerno_databases_open {databases}"
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_format_holds_every_family() {
        let m = Metrics::default();
        m.http(200, Duration::from_micros(500));
        m.http(200, Duration::from_millis(30));
        m.http(401, Duration::from_secs(9));
        m.grpc("ok", Duration::from_millis(2));
        m.grpc("not_found", Duration::from_millis(2));
        m.auth_failure();
        let text = m.render(3, None);
        for line in [
            "# TYPE kerno_http_responses_total counter",
            "kerno_http_responses_total{status=\"200\"} 2",
            "kerno_http_responses_total{status=\"401\"} 1",
            "kerno_grpc_calls_total{code=\"ok\"} 1",
            "kerno_grpc_calls_total{code=\"not_found\"} 1",
            "kerno_auth_failures_total 1",
            "# TYPE kerno_request_duration_seconds histogram",
            // 0.5 ms is in the first bucket, 30 ms from the 0.05 bucket on, 9 s only in +Inf.
            "kerno_request_duration_seconds_bucket{transport=\"http\",le=\"0.001\"} 1",
            "kerno_request_duration_seconds_bucket{transport=\"http\",le=\"0.025\"} 1",
            "kerno_request_duration_seconds_bucket{transport=\"http\",le=\"0.05\"} 2",
            "kerno_request_duration_seconds_bucket{transport=\"http\",le=\"5\"} 2",
            "kerno_request_duration_seconds_bucket{transport=\"http\",le=\"+Inf\"} 3",
            "kerno_request_duration_seconds_count{transport=\"http\"} 3",
            "kerno_request_duration_seconds_bucket{transport=\"grpc\",le=\"0.0025\"} 2",
            "kerno_request_duration_seconds_count{transport=\"grpc\"} 2",
            "kerno_databases_open 3",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
        }
        let sum: f64 = text
            .lines()
            .find_map(|l| l.strip_prefix("kerno_request_duration_seconds_sum{transport=\"http\"} "))
            .unwrap()
            .parse()
            .unwrap();
        assert!((sum - 9.0305).abs() < 1e-9, "{sum}");
    }

    #[test]
    fn the_backup_families_are_there_only_when_backups_are_on() {
        let m = Metrics::default();
        assert!(!m.render(1, None).contains("kerno_backup"));
        let text = m.render(1, Some(BackupTotals { commits: 7, segments: 3, bases: 2, errors: 1 }));
        for line in [
            "kerno_backup_shipped_commits_total 7",
            "kerno_backup_segments_total 3",
            "kerno_backup_bases_total 2",
            "kerno_backup_errors_total 1",
            "# TYPE kerno_backup_errors_total counter",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
        }
    }
}
