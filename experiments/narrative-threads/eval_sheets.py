"""Blind sheets for the independent human evaluation.

`results/eval/ANSWERS.md`: per question, its reference answer and the three strategies' answers
labelled A, B, C in an order fixed by a hash of the question id (no strategy name shown), each
with the passages it was given. The reviewer marks, per answer: coherent narrative, evidence
supports it, invented evidence, unnecessary context. The label -> strategy key goes to
`results/eval/KEY.json`, which the reviewer must not open before finishing.

`results/eval/RELATIONS.md`: a sample of accepted classified relations (every 1 in `STEP`, in a
fixed order) with their quotes, each marked justifiable or not.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from segment import segment
from validate import validate

HERE = Path(__file__).resolve().parent
EVAL = HERE / "results" / "eval"
STEP = 4


def order(qid: str, names: list[str]) -> list[str]:
    return sorted(names, key=lambda n: hashlib.sha256(f"{qid}:{n}".encode()).hexdigest())


def answers_sheet(extractor: str = "x2") -> None:
    rows = [json.loads(x) for x in (HERE / "results" / "answers" / f"{extractor}.jsonl").read_text().splitlines()]
    blocks_of = {(r["question"], r["strategy"]): r["blocks"] for r in
                 map(json.loads, (HERE / "results" / "retrieval" / f"{extractor}.jsonl").read_text().splitlines())}
    frozen, docs = {}, {}
    for p in (HERE / "results" / "questions" / "frozen").glob("*.json"):
        if p.name != "MANIFEST.json":
            d = json.loads(p.read_text())
            for q in d["questions"]:
                frozen[q["id"]] = (q, d["document"])
    by_q: dict[str, dict] = {}
    for r in rows:
        by_q.setdefault(r["question"], {})[r["strategy"]] = r
    out, key = ["# Blind answer evaluation", "", "Do not open KEY.json before finishing.", ""], {}
    for qid in sorted(by_q):
        q, doc = frozen[qid]
        if doc not in docs:
            data = (HERE / "corpus" / doc).read_bytes()
            docs[doc] = (data, segment(data))
        data, blocks = docs[doc]
        out += [f"## {qid} {q['question']}", "", f"**Reference:** {q['reference_answer']}", ""]
        key[qid] = {}
        for label, s in zip("ABC", order(qid, sorted(by_q[qid]))):
            key[qid][label] = s
            a = by_q[qid][s]
            out += [f"### {label}", "", a["answer"] or "", "", "<details><summary>passages given</summary>", ""]
            out += [f"- b{i}: {' '.join(data[blocks[i].start:blocks[i].end].decode('utf-8').split())}"
                    for i in sorted(blocks_of[(qid, s)])]
            out += ["", "</details>", "",
                    "- [ ] coherent narrative  - [ ] evidence supports it  - [ ] invented evidence  - [ ] unnecessary context", ""]
    EVAL.mkdir(parents=True, exist_ok=True)
    (EVAL / "ANSWERS.md").write_text("\n".join(out))
    (EVAL / "KEY.json").write_text(json.dumps(key, indent=1) + "\n")


def relations_sheet(extractor: str = "x2") -> None:
    out = ["# Relation justifiability (sample)", ""]
    for p in sorted((HERE / "results" / "extraction" / extractor).glob("*.json")):
        d = json.loads(p.read_text())
        data = (HERE / "corpus" / d["document"]).read_bytes()
        rep = validate(data, segment(data), d)
        topics = {s["id"]: s["topic"] for s in rep.segments}
        rels = [r for r in rep.relations if r["kind"] != "unclassified"][::STEP]
        out += [f"## {d['document']} ({len(rels)} sampled)", ""]
        for r in rels:
            out += [f"### {r['from']} ({topics[r['from']]}) --{r['kind']}--> {r['to']} ({topics[r['to']]})", ""]
            out += [f"- `{e['span'][0]}:{e['span'][1]}` {' '.join(e['quote'].split())}" for e in r["evidence"]]
            out += ["", "- [ ] justifiable  - [ ] not justifiable", ""]
    EVAL.mkdir(parents=True, exist_ok=True)
    (EVAL / "RELATIONS.md").write_text("\n".join(out))


if __name__ == "__main__":
    import sys
    relations_sheet()
    if "--answers" in sys.argv:
        answers_sheet()
