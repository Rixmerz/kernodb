"""The owner's delegated review (2026-11-09: "No voy a revisarlo, hazlo tú"), done by a model that
neither wrote the questions (Haiku) nor built the threads (Sonnet): Opus, isolated
(`model_call`), charged to the ledger's `eval` phase. This replaces the independent human of
the original protocol, and every output says so (`reviewer`).

- `questions`: per document, each draft question with its reference answer and its evidence as
  it stands in the corpus; the reviewer decides keep / edit / drop and whether the evidence
  justifies the answer. Its marks are written into REVIEW.md, so `freeze.py` reads them as it
  would a person's.
- `relations`: each sampled relation of RELATIONS.md, justifiable or not.

A run stops when the ledger refuses a call and keeps what it decided.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

from ledger import CapReached, Ledger
from model_call import answer_of, ask

HERE = Path(__file__).resolve().parent
ALIAS = "opus"
QDIR = HERE / "results" / "questions"
EVAL = HERE / "results" / "eval"

Q_SCHEMA = {"type": "object", "properties": {"decisions": {"type": "array", "items": {"type": "object", "properties": {
    "id": {"type": "string"}, "decision": {"type": "string", "enum": ["keep", "edit", "drop"]},
    "question": {"type": "string"}, "reference_answer": {"type": "string"},
    "evidence_justifies": {"type": "boolean"}, "reason": {"type": "string"}},
    "required": ["id", "decision", "evidence_justifies", "reason"]}}}, "required": ["decisions"]}

Q_PROMPT = """You are the independent reviewer of a retrieval benchmark. For each question below you
see its type, its reference answer and the evidence passages that are meant to justify it.
Decide:
- keep: clear, answerable from the evidence, and the reference answer is supported by it;
- edit: fixable by rewording the question or the reference answer so that the evidence supports it
  (give the new text in "question" and/or "reference_answer"; the evidence cannot change);
- drop: the evidence does not support an answer, the question is ambiguous or trivial, or it has
  no evidence.
Also say whether the evidence justifies the (possibly edited) answer, with a one-line reason.
Trajectory, contrast, causal and evolution questions need evidence from more than one place.
Answer only with the JSON object, one decision per question id.

QUESTIONS:
"""

R_SCHEMA = {"type": "object", "properties": {"verdicts": {"type": "array", "items": {"type": "object", "properties": {
    "index": {"type": "integer"}, "justifiable": {"type": "boolean"}, "reason": {"type": "string"}},
    "required": ["index", "justifiable", "reason"]}}}, "required": ["verdicts"]}

R_PROMPT = """You are the independent reviewer of relations an extractor drew between segments of a
document. Each relation below names its kind and quotes the passages offered as evidence. A
relation is justifiable when the quotes, read as written, show that kind of link between those
two segments (develops, contradicts, anticipates, resumes, exemplifies, resolves, depends_on);
sharing a theme is not enough. Answer only with the JSON object, one verdict per index.

RELATIONS:
"""


def sections(sheet: str, level: str) -> list[tuple[str, str]]:
    """(heading line, body) for each heading of `level` ("## " or "### ")."""
    parts = re.split(rf"(?m)^(?={re.escape(level)})", sheet)
    return [(p.splitlines()[0], p) for p in parts if p.startswith(level)]


def checkbox_line(decision: str, justifies: bool) -> str:
    box = {k: "x" if k == decision else " " for k in ("keep", "edit", "drop")}
    return (f"- [{box['keep']}] keep  - [{box['edit']}] edit  - [{box['drop']}] drop  "
            f"- [{'x' if justifies else ' '}] evidence justifies the answer")


def apply(sheet: str, decisions: dict[str, dict]) -> str:
    out = []
    for block in re.split(r"(?m)^(?=### )", sheet):
        m = re.match(r"### (\w+) \[(\w+)\] (.*)", block)
        d = decisions.get(m.group(1)) if m else None
        if d:
            if d["decision"] == "edit" and d.get("question"):
                block = block.replace(m.group(0), f"### {m.group(1)} [{m.group(2)}] {d['question'].strip()}", 1)
            if d["decision"] == "edit" and d.get("reference_answer"):
                block = re.sub(r"(?m)^\*\*Reference:\*\* .*$",
                               lambda _: f"**Reference:** {d['reference_answer'].strip()}", block, count=1)
            block = re.sub(r"(?m)^- \[.\] keep .*$", lambda _: checkbox_line(d["decision"], d["evidence_justifies"]),
                           block, count=1)
            block = block.rstrip("\n") + f"\n\n> reviewer ({d['reviewer']}): {d['reason']}\n\n"
        out.append(block)
    return "".join(out)


def review_questions(ledger: Ledger, pinned: dict) -> None:
    path = EVAL / "question_review.json"
    done = json.loads(path.read_text()) if path.exists() else {}
    sheet = (QDIR / "REVIEW.md").read_text()
    for head, body in sections(sheet, "## "):
        doc = head[3:].split(" (")[0]
        if doc in done or doc.startswith("Question review"):
            continue
        ledger.authorize("eval")
        result = ask(ALIAS, Q_SCHEMA, Q_PROMPT + body, timeout=1200)
        entry = ledger.record("eval", doc, ALIAS, result, f"review:questions:{doc}")
        if pinned.setdefault("ids", entry["model_ids"]) != entry["model_ids"]:
            raise SystemExit("model changed mid-experiment; stopping")
        reviewer = ",".join(entry["model_ids"])
        done[doc] = {d["id"]: {**d, "reviewer": reviewer} for d in answer_of(result)["decisions"]}
        EVAL.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(done, indent=1, ensure_ascii=False) + "\n")
        print(f"{doc}: {len(done[doc])} decided, USD {entry['charged_usd']:.3f}, eval {ledger.spent('eval'):.3f}",
              file=sys.stderr)
    merged = {k: v for per in done.values() for k, v in per.items()}
    (QDIR / "REVIEW.md").write_text(apply(sheet, merged))


def review_relations(ledger: Ledger, pinned: dict) -> None:
    path = EVAL / "relation_review.json"
    if path.exists():
        return
    items = [b for _, b in sections((EVAL / "RELATIONS.md").read_text(), "### ")]
    text = "\n".join(f"[{i}] {b.split('- [ ] justifiable')[0].strip()}" for i, b in enumerate(items))
    ledger.authorize("eval")
    result = ask(ALIAS, R_SCHEMA, R_PROMPT + text, timeout=1200)
    entry = ledger.record("eval", "all", ALIAS, result, "review:relations")
    if pinned.setdefault("ids", entry["model_ids"]) != entry["model_ids"]:
        raise SystemExit("model changed mid-experiment; stopping")
    verdicts = answer_of(result)["verdicts"]
    ok = sum(v["justifiable"] for v in verdicts)
    path.write_text(json.dumps({"reviewer": entry["model_ids"], "sampled": len(items), "judged": len(verdicts),
                                "justifiable": ok, "verdicts": verdicts}, indent=1, ensure_ascii=False) + "\n")
    print(f"relations: {ok} of {len(verdicts)} justifiable ({len(items)} sampled), USD {entry['charged_usd']:.3f}",
          file=sys.stderr)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("what", choices=["questions", "relations"])
    args = ap.parse_args()
    ledger, pinned = Ledger(), {}
    try:
        (review_questions if args.what == "questions" else review_relations)(ledger, pinned)
    except CapReached as e:
        print(f"stopped: {e}; partial review kept", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
