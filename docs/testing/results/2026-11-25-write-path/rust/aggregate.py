"""Before/after tables of the Rust storage harness (spike-bench suite), base ea8b610 vs head.

Usage: python3 -I aggregate.py tables > tables.md
       python3 -I aggregate.py runs > per-run.md
Reads {base,head}-{sync,buffered}-R.jsonl from this directory (run from here).
Cells: median of the runs, with (min-max) when the range is over 15% of the median (ADR-0012).
Ratio = kerno / sqlite, computed inside each run (same process round), then the median.
`worse` marks a metric where head is worse than base with disjoint ranges; `better` the opposite.
Headline metrics are the list of spikes/storage/aggregate-sqlite.py.
"""
import glob
import json
import statistics as st
import sys

HEADLINE = {
    "bulk_load": ["load_kops", "write_amp", "space_amp_closed", "close_ms", "reopen_ms", "recovery_ms", "peak_rss_mb"],
    "point_reads": ["get_1t_kops", "get_1t_p50_us", "get_1t_p95_us", "get_1t_p99_us", "get_4t_kops", "get_4t_p99_us"],
    "range_scans": ["scan100_kops", "scan100_p50_us", "scan100_p95_us", "scan100_p99_us"],
    "small_commits": ["commits_per_s", "commit_p50_us", "commit_p95_us", "commit_p99_us", "commit_p999_us", "write_amp", "commit_fsyncs_per_commit"],
    "point_deletes": ["delete_commits_per_s", "delete_commit_p50_us", "delete_commit_p99_us", "get_deleted_p50_us", "get_deleted_p99_us", "get_live_p99_us"],
    "append_seq": ["append_kops", "update_kops", "update_commit100_p99_us", "write_amp", "space_amp", "peak_rss_mb"],
    "mixed": ["mixed_kops", "read_p50_us", "read_p99_us", "write_p50_us", "write_p99_us"],
    "history_sim": ["revisions_per_s_k", "rev_commit100_p99_us", "space_amp", "history_scan_p50_us", "history_scan_p99_us", "reconstruct_p99_us"],
    "livespec": ["changesets_per_s", "changeset_commit_p50_us", "changeset_commit_p99_us", "symbol_get_p99_us", "module_scan_p50_us", "module_scan_p99_us"],
    "cauce": ["steps_per_s", "step_commit_p50_us", "step_commit_p99_us", "step_read_p99_us", "recovery_ms"],
    "agent_memory": ["steps_per_s", "memory_write_p50_us", "memory_write_p99_us", "recall_p99_us", "recent_scan_p99_us", "expire_commit_p99_us", "space_amp"],
    "concurrent_commits": ["cc_threads", "cc_commits_per_s", "cc_commit_p50_us", "cc_commit_p99_us", "cc_fsyncs_per_commit"],
}


def higher_is_better(metric):
    return "kops" in metric or metric.endswith("_per_s") or metric.endswith("per_s_k")


# (durability, n, workload, metric, side) -> list of (round, kerno, sqlite)
data = {}
errors = []
for path in sorted(glob.glob("*-*-*.jsonl")):
    side, dur, rnd = path[: -len(".jsonl")].split("-")
    rows = {}
    for line in open(path, encoding="utf-8"):
        row = json.loads(line)
        if "error" in row:
            errors.append((path, row.get("workload"), row.get("engine"), row["error"].strip().splitlines()[-1][:160]))
            continue
        rows[(int(row["n"]), row["workload"], row["engine"])] = row
    for (n, wl, eng), row in rows.items():
        if eng != "kerno" or (n, wl, "sqlite") not in rows:
            continue
        sq = rows[(n, wl, "sqlite")]
        for m in HEADLINE[wl]:
            if m in row and m in sq:
                data.setdefault((dur, n, wl, m, side), []).append((int(rnd), row[m], sq[m]))


def cell(v):
    med = st.median(v)
    rng = f" ({min(v):.3g}-{max(v):.3g})" if med and (max(v) - min(v)) > 0.15 * abs(med) else ""
    return f"{med:.3g}{rng}"


def ratio(k, s):
    return k / s if s else float("nan")


def keys():
    seen = sorted({k[:4] for k in data}, key=lambda k: (k[0] != "sync", -k[1], list(HEADLINE).index(k[2]), HEADLINE[k[2]].index(k[3])))
    return [k for k in seen if k + ("base",) in data and k + ("head",) in data]


def tables():
    if errors:
        print("Errors:", *errors, sep="\n- ")
    counts = {}
    cur = None
    for dur, n, wl, m in keys():
        if (dur, n) != cur:
            cur = (dur, n)
            print(f"\n### {dur}, n = {n}\n")
            print("| workload | metric | runs | kerno before | kerno after | after/before | sqlite | ratio before | ratio after | change |")
            print("|---|---|---|---|---|---|---|---|---|---|")
        b, h = data[(dur, n, wl, m, "base")], data[(dur, n, wl, m, "head")]
        kb, kh = [x[1] for x in b], [x[1] for x in h]
        sq = [x[2] for x in b + h]
        rb, rh = [ratio(x[1], x[2]) for x in b], [ratio(x[1], x[2]) for x in h]
        hib = higher_is_better(m)
        if max(kh) < min(kb):
            change = "better" if not hib else "worse"
        elif min(kh) > max(kb):
            change = "worse" if not hib else "better"
        else:
            change = "overlap"
        if m == "cc_threads":
            change = "load"
        counts.setdefault((dur, n), {}).setdefault(change, 0)
        counts[(dur, n)][change] += 1
        ab = st.median(kh) / st.median(kb) if st.median(kb) else float("nan")
        print(f"| {wl} | `{m}` | {len(b)}/{len(h)} | {cell(kb)} | {cell(kh)} | {ab:.2f} | {cell(sq)} | {cell(rb)} | {cell(rh)} | {change} |")
    print("\n### Summary (head against base, kerno only; disjoint ranges only count as better or worse)\n")
    for (dur, n), c in counts.items():
        print(f"- {dur}, n = {n}: " + ", ".join(f"{k} {v}" for k, v in sorted(c.items())))


def runs():
    print("Raw value of every run. Columns per run R: kerno base, kerno head, sqlite in the base run, sqlite in the head run.\n")
    cur = None
    for dur, n, wl, m in keys():
        if (dur, n) != cur:
            cur = (dur, n)
            print(f"\n### {dur}, n = {n}\n")
            print("| workload | metric | kerno base (R1..R5) | kerno head (R1..R5) | sqlite base runs | sqlite head runs |")
            print("|---|---|---|---|---|---|")
        b = sorted(data[(dur, n, wl, m, "base")])
        h = sorted(data[(dur, n, wl, m, "head")])
        f = lambda xs, i: " ".join(f"{x[i]:.4g}" for x in xs)
        print(f"| {wl} | `{m}` | {f(b, 1)} | {f(h, 1)} | {f(b, 2)} | {f(h, 2)} |")


{"tables": tables, "runs": runs}[sys.argv[1]]()
