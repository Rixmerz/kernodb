#!/usr/bin/env python3
"""Tables for the Kerno vs SQLite comparison from the raw JSONL that
`run-sqlite-comparison.sh` leaves in a results directory.

Usage: python3 -I spikes/storage/aggregate-sqlite.py RESULTS_DIR > tables.md

For every workload and durability pair: the median [min-max] of each headline metric over the
repetitions, and the ratio of the medians (kerno / sqlite). Then one summary that counts, per
pair, the metrics where kerno is better, worse, or not distinguishable (the ranges overlap),
and lists every metric where kerno is worse with disjoint ranges. Nothing is left out.
"""
import collections
import json
import statistics
import sys
from pathlib import Path

# Headline metrics per workload (the list of `headline()` in spike-bench/src/main.rs).
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
PAIRS = {
    "sync": "kerno `Sync` against SQLite WAL + `synchronous=FULL` (the equivalent pair)",
    "buffered": "kerno `Buffered` against SQLite WAL + `synchronous=NORMAL` (NOT equivalent guarantees)",
}


def higher_is_better(metric: str) -> bool:
    """Throughput metrics grow when the engine is faster; everything else (latency, time, amplification,
    memory, fsyncs per commit) shrinks. `cc_threads` is the load, not a result."""
    return "kops" in metric or metric.endswith("_per_s") or metric.endswith("per_s_k")


def load(results_dir: Path, durability: str) -> tuple[dict, list]:
    data = collections.defaultdict(lambda: collections.defaultdict(list))  # (workload, engine) -> metric -> values
    errors = []
    path = results_dir / f"results-{durability}.jsonl"
    if not path.exists():
        return data, errors
    with path.open(encoding="utf-8") as f:
        for line in f:
            try:
                row = json.loads(line)
            except ValueError:
                continue  # a line cut by an interrupted run
            if "error" in row:
                errors.append((row.get("workload"), row.get("engine"), row["error"].strip().splitlines()[-1][:160]))
                continue
            for metric, value in row.items():
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    data[(row.get("workload"), row["engine"])][metric].append(float(value))
    return data, errors


def fmt(x: float) -> str:
    return f"{x:.0f}" if x >= 100 else f"{x:.1f}" if x >= 10 else f"{x:.2f}"


def cell(values: list[float]) -> str:
    if not values:
        return "n/a"
    return f"{fmt(statistics.median(values))} [{fmt(min(values))}-{fmt(max(values))}]"


def ratio(a: list[float], b: list[float]) -> str:
    if not a or not b or not statistics.median(b):
        return "n/a"
    r = statistics.median(a) / statistics.median(b)
    return "<0.01" if r < 0.005 else f"{r:.2f}"


def verdict(metric: str, kerno: list[float], sqlite: list[float]) -> str | None:
    """'better', 'worse' or 'tie' for kerno; a tie when the ranges overlap."""
    if not kerno or not sqlite or metric == "cc_threads":
        return None
    if not (max(kerno) < min(sqlite) or max(sqlite) < min(kerno)):
        return "tie"
    kerno_lower = statistics.median(kerno) < statistics.median(sqlite)
    return "better" if kerno_lower != higher_is_better(metric) else "worse"


def repetitions(runs: dict | None) -> int:
    return len(runs["wall_s"]) if runs else 0


def main(results_dir: Path) -> None:
    summary = {}
    losses = []
    for durability, pair in PAIRS.items():
        data, errors = load(results_dir, durability)
        counts = collections.Counter()
        print(f"## Durability `{durability}`: {pair}\n")
        for workload, metrics in HEADLINE.items():
            kerno, sqlite = data.get((workload, "kerno")), data.get((workload, "sqlite"))
            if not kerno and not sqlite:
                continue
            print(f"### `{workload}` ({durability}; repetitions: kerno {repetitions(kerno)}, sqlite {repetitions(sqlite)})\n")
            print("| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |")
            print("|---|---|---|---|---|")
            for metric in metrics:
                a, b = (kerno[metric] if kerno else []), (sqlite[metric] if sqlite else [])
                if not a and not b:
                    continue
                v = verdict(metric, a, b)
                if v:
                    counts[v] += 1
                if v == "worse":
                    losses.append((durability, workload, metric, cell(a), cell(b), ratio(a, b)))
                print(f"| `{metric}` | {cell(a)} | {cell(b)} | {ratio(a, b)} | {v or 'n/a'} |")
            print()
        for error in errors:
            print(f"- Failed run recorded: {error}")
        summary[durability] = counts
    print("## Summary\n")
    print("| Pair | kerno better | not distinguishable (ranges overlap) | kerno worse |")
    print("|---|---|---|---|")
    for durability, counts in summary.items():
        print(f"| `{durability}` | {counts['better']} | {counts['tie']} | {counts['worse']} |")
    print("\n### Every metric where kerno is worse and the ranges do not overlap\n")
    if losses:
        print("| Pair | Workload | Metric | kerno | sqlite | kerno / sqlite |")
        print("|---|---|---|---|---|---|")
        for row in losses:
            print("| " + " | ".join(f"`{x}`" if i == 2 else x for i, x in enumerate(row)) + " |")
    else:
        print("None.")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(Path(sys.argv[1]))
