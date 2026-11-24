"""Close, reopen and disk of the Python benchmark (vs_sqlite.py, 100k units), base vs head.

Usage: python3 -I aggregate-python.py   (reads python/base-N.json and python/head-N.json)
"""
import glob
import json
import statistics as st

data = {}
for f in sorted(glob.glob("python/base-*.json") + glob.glob("python/head-*.json")):
    side = f.split("/")[1].split("-")[0]
    for run in json.load(open(f))["runs"]:
        for row in ("close", "reopen", "disk MiB"):
            for engine in ("kerno", "sqlite"):
                data.setdefault((row, engine, side), []).append(run[row][engine])


def cell(v, unit):
    med = st.median(v)
    rng = f" ({min(v):.4g}-{max(v):.4g})" if (max(v) - min(v)) > 0.15 * med else ""
    return f"{med:.4g}{rng}"


print("| row | engine | runs | before (base 9f8c60a) | after (head) |")
print("|---|---|---|---|---|")
for row in ("close", "reopen", "disk MiB"):
    for engine in ("kerno", "sqlite"):
        b, h = data[(row, engine, "base")], data[(row, engine, "head")]
        unit = "MiB" if row == "disk MiB" else "s"
        print(f"| {row} ({unit}) | {engine} | {len(b)} / {len(h)} | {cell(b, unit)} | {cell(h, unit)} |")
