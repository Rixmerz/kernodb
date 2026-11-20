"""Retrieval metrics of the three strategies over the frozen questions.

Refuses to run unless `results/questions/frozen/` exists and every file matches the SHA-256 in
its MANIFEST: the strategies never see a question before it is frozen.

Per question and strategy: the retrieved blocks (under the same byte budget), evidence
coverage (share of the question's evidence ranges that overlap a retrieved block) and
precision (share of retrieved blocks that overlap any evidence range). Written to
`results/retrieval/<extractor>.jsonl` with the configuration, and summarised per strategy.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

from retrieve import CONFIG, STRATEGIES, load, query_vector

HERE = Path(__file__).resolve().parent
FROZEN = HERE / "results" / "questions" / "frozen"


def overlaps(a: tuple[int, int], b: tuple[int, int]) -> bool:
    return a[0] < b[1] and b[0] < a[1]


def score(evidence: list[dict], spans: list[tuple[int, int]]) -> dict:
    ev = [(e["start"], e["end"]) for e in evidence]
    covered = sum(any(overlaps(e, s) for s in spans) for e in ev)
    useful = sum(any(overlaps(s, e) for e in ev) for s in spans)
    return {"coverage": covered / len(ev) if ev else 0.0, "precision": useful / len(spans) if spans else 0.0}


def frozen_files() -> list[Path]:
    man = json.loads((FROZEN / "MANIFEST.json").read_text())
    for name, info in man["files"].items():
        if hashlib.sha256((FROZEN / name).read_bytes()).hexdigest() != info["sha256"]:
            raise SystemExit(f"{name} does not match its frozen hash")
    return [FROZEN / n for n in sorted(man["files"])]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--extractor", default="x2")
    args = ap.parse_args()
    if not (FROZEN / "MANIFEST.json").exists():
        print("questions are not frozen yet (freeze.py after the human review)", file=sys.stderr)
        return 1
    out = HERE / "results" / "retrieval" / f"{args.extractor}.jsonl"
    out.parent.mkdir(parents=True, exist_ok=True)
    totals: dict[str, list[dict]] = {k: [] for k in STRATEGIES}
    with out.open("w") as fh:
        for path in frozen_files():
            qs = json.loads(path.read_text())
            name = qs["document"]
            ext = HERE / "results" / "extraction" / args.extractor / f"{Path(name).stem}.json"
            data, blocks, vecs, graph = load(name, ext)
            for q in qs["questions"]:
                sims = vecs @ query_vector(q["question"])
                for sname, f in STRATEGIES.items():
                    got = f(sims, blocks, graph)
                    m = score(q["evidence"], [(blocks[i].start, blocks[i].end) for i in got])
                    totals[sname].append(m)
                    fh.write(json.dumps({"question": q["id"], "document": name, "strategy": sname,
                                         "blocks": got, **m, "config": CONFIG, "extractor": args.extractor}) + "\n")
    for sname, ms in totals.items():
        n = len(ms)
        print(f"{sname:9s} n={n} coverage={sum(m['coverage'] for m in ms) / n:.3f} "
              f"precision={sum(m['precision'] for m in ms) / n:.3f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
