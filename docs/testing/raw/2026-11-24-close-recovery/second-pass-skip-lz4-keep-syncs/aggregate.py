"""Median (and min-max) of close_ms, reopen_ms and recovery_ms per side, engine, scale and workload.

Usage: python3 -I aggregate.py   (reads base-*.jsonl and head-*.jsonl in this directory)
"""
import glob
import json
import statistics as st

rows = []
for f in sorted(glob.glob("base-*.jsonl") + glob.glob("head-*.jsonl")):
    side = f.split("-")[0]
    for line in open(f):
        r = json.loads(line)
        r["side"] = side
        rows.append(r)

metrics = {"bulk_load": ["close_ms", "reopen_ms", "recovery_ms"], "cauce": ["recovery_ms"]}


def cell(vals):
    med = st.median(vals)
    lo, hi = min(vals), max(vals)
    wide = (hi - lo) > 0.15 * med
    return f"{med:.1f}" + (f" ({lo:.1f}-{hi:.1f})" if wide else "")


print("| scale | workload | metric | engine | runs | before (base 9f8c60a) | after (head) |")
print("|---|---|---|---|---|---:|---:|")
for n in (10000, 100000):
    for wl, ms in metrics.items():
        for m in ms:
            for engine in ("kerno", "sqlite"):
                get = lambda side: [r[m] for r in rows if r["side"] == side and r["engine"] == engine and r["n"] == n and r["workload"] == wl and m in r]
                b, h = get("base"), get("head")
                if b and h:
                    print(f"| {n} | {wl} | `{m}` | {engine} | {len(b)} / {len(h)} | {cell(b)} | {cell(h)} |")
print()
print("Load average (1 min) at the start of each run: min %.2f, max %.2f" % (min(r["load1"] for r in rows), max(r["load1"] for r in rows)))
