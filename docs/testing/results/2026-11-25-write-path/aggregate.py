"""Before/after table of the Python benchmark (vs_sqlite.py), base ea8b610 vs head.

Usage: python3 -I aggregate.py   (reads python/{base,head}-N-R.json; run from this directory)
Cells: median of the runs, with (min-max) when the range is over 15% of the median (ADR-0012).
Ratio = kerno / sqlite, computed inside each run, then the median of the runs.
"""
import glob
import json
import statistics as st

data = {}
for f in sorted(glob.glob("python/*.json")):
    side, n, _ = f.split("/")[1].split("-")
    for run in json.load(open(f))["runs"]:
        for row, v in run.items():
            data.setdefault((int(n), row, side), []).append((v["kerno"], v["sqlite"]))


def cell(v):
    med = st.median(v)
    rng = f" ({min(v):.3g}-{max(v):.3g})" if (max(v) - min(v)) > 0.15 * med else ""
    return f"{med:.3g}{rng}"


for n in (10000, 100000):
    print(f"\n### n = {n}\n")
    print("| row | runs | kerno before | kerno after | after/before | sqlite | ratio before | ratio after |")
    print("|---|---|---|---|---|---|---|---|")
    rows = [r for (m, r, s) in data if m == n and s == "base"]
    for row in dict.fromkeys(rows):
        b, h = data[(n, row, "base")], data[(n, row, "head")]
        kb, kh = [x[0] for x in b], [x[0] for x in h]
        sq = [x[1] for x in b + h]
        rb, rh = [x[0] / x[1] for x in b], [x[0] / x[1] for x in h]
        print(f"| {row} | {len(b)}/{len(h)} | {cell(kb)} | {cell(kh)} | {st.median(kh) / st.median(kb):.2f} | {cell(sq)} | {cell(rb)} | {cell(rh)} |")
