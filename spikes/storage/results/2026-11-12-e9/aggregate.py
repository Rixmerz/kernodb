#!/usr/bin/env python3
"""Per-workload tables (median [min-max] over the repetitions) for kerno vs
sqlite from the raw JSONL files in this directory. Usage: python3 -I aggregate.py > tables.md"""
import json, statistics, collections, os

HERE = os.path.dirname(os.path.abspath(__file__))
# Headline metrics per workload (same list as `headline()` in spike-bench/src/main.rs).
H = {
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
# Files per durability.
FILES = {"sync": ["results-sync.jsonl"], "buffered": ["results-buffered.jsonl"]}

def load(dur):
    data = collections.defaultdict(lambda: collections.defaultdict(list))  # (wl,engine) -> metric -> values
    errors = []
    for f in FILES[dur]:
        p = os.path.join(HERE, f)
        if not os.path.exists(p):
            continue
        for line in open(p):
            try:
                v = json.loads(line)
            except ValueError:
                continue  # defensive: skip any line that is not valid JSON
            wl = v.get("workload")
            if "error" in v:
                errors.append((wl, v["engine"], v["error"].strip().splitlines()[-1][:120]))
                continue
            for m, x in v.items():
                if isinstance(x, (int, float)) and not isinstance(x, bool):
                    data[(wl, v["engine"])][m].append(float(x))
    return data, errors

def fmt(x):
    return f"{x:.0f}" if x >= 100 else f"{x:.1f}" if x >= 10 else f"{x:.2f}"

def cell(vs):
    if not vs:
        return "—"
    return f"{fmt(statistics.median(vs))} [{fmt(min(vs))}–{fmt(max(vs))}]"

def ratio(a, b):
    if not a or not b:
        return "—"
    ma, mb = statistics.median(a), statistics.median(b)
    if not mb:
        return "—"
    r = ma / mb
    return "<0.01" if r < 0.005 else f"{r:.2f}"

for dur in ("sync", "buffered"):
    data, errors = load(dur)
    pair = "kerno `Sync` frente a SQLite WAL + `synchronous=FULL`" if dur == "sync" else "kerno `Buffered` frente a SQLite WAL + `synchronous=NORMAL`"
    print(f"## Durabilidad `{dur}`: {pair}\n")
    for wl, ms in H.items():
        k, s = data.get((wl, "kerno")), data.get((wl, "sqlite"))
        if not k and not s:
            continue
        nk = len(k["wall_s"]) if k else 0
        ns = len(s["wall_s"]) if s else 0
        print(f"### `{wl}` ({dur}; repeticiones: kerno {nk}, sqlite {ns})\n")
        print("| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |")
        print("|---|---|---|---|")
        for m in ms:
            a = k[m] if k else []
            b = s[m] if s else []
            if not a and not b:
                continue
            print(f"| `{m}` | {cell(a)} | {cell(b)} | {ratio(a, b)} |")
        print()
    for e in errors:
        print(f"- Fallo registrado: {e}")
