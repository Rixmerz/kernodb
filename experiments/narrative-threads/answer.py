"""Answers with Haiku over each strategy's retrieved context (ledger phase `queries`, USD 7).

Reads `results/retrieval/<extractor>.jsonl` (written only after the freeze). Each call gets the
question and the retrieved blocks, labelled [bN], and must answer only from them, citing the
block ids it used; a cited id that was not in its context is recorded (`cited_outside`), an
objective sign of invented evidence. Calls are isolated (`model_call`); a run stops at the cap
and keeps what it answered.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from ledger import CapReached, Ledger
from model_call import answer_of, ask
from segment import segment

HERE = Path(__file__).resolve().parent
ALIAS = "haiku"
SCHEMA = {"type": "object", "properties": {
    "answer": {"type": "string"}, "cited_blocks": {"type": "array", "items": {"type": "string"}},
    "insufficient": {"type": "boolean"}}, "required": ["answer", "cited_blocks", "insufficient"]}
PROMPT = """Answer the question using ONLY the passages below. Cite the ids of the passages your
answer relies on. If they do not contain enough to answer, say so and set insufficient=true.
Do not use outside knowledge. Answer only with the JSON object.

QUESTION: {question}

PASSAGES:
{passages}
"""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--extractor", default="x2")
    args = ap.parse_args()
    rows = [json.loads(x) for x in (HERE / "results" / "retrieval" / f"{args.extractor}.jsonl").read_text().splitlines()]
    frozen = {}
    for p in (HERE / "results" / "questions" / "frozen").glob("*.json"):
        if p.name != "MANIFEST.json":
            for q in json.loads(p.read_text())["questions"]:
                frozen[q["id"]] = q
    out = HERE / "results" / "answers" / f"{args.extractor}.jsonl"
    out.parent.mkdir(parents=True, exist_ok=True)
    done = {(r["question"], r["strategy"]) for r in map(json.loads, out.read_text().splitlines())} if out.exists() else set()
    ledger, pinned, cache = Ledger(), {}, {}
    for row in rows:
        key = (row["question"], row["strategy"])
        if key in done:
            continue
        if row["document"] not in cache:
            data = (HERE / "corpus" / row["document"]).read_bytes()
            cache[row["document"]] = (data, segment(data))
        data, blocks = cache[row["document"]]
        passages = "\n\n".join(f"[b{i}] {data[blocks[i].start:blocks[i].end].decode('utf-8')}" for i in sorted(row["blocks"]))
        try:
            ledger.authorize("queries")
        except CapReached as e:
            print(f"stopped: {e}; partial answers kept", file=sys.stderr)
            return 2
        result = ask(ALIAS, SCHEMA, PROMPT.format(question=frozen[row["question"]]["question"], passages=passages), timeout=600)
        entry = ledger.record("queries", row["document"], ALIAS, result,
                              f"answer:{args.extractor}:{row['question']}:{row['strategy']}")
        if pinned.setdefault("ids", entry["model_ids"]) != entry["model_ids"]:
            raise SystemExit("model changed mid-experiment; stopping")
        a = answer_of(result)
        given = {f"b{i}" for i in row["blocks"]}
        with out.open("a") as fh:
            fh.write(json.dumps({"question": row["question"], "strategy": row["strategy"], "answer": a.get("answer"),
                                 "cited_blocks": a.get("cited_blocks"), "insufficient": a.get("insufficient"),
                                 "cited_outside": sorted(set(a.get("cited_blocks") or []) - given),
                                 "model_ids": entry["model_ids"]}, ensure_ascii=False) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
