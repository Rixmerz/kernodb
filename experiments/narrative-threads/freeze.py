"""Freezes the reviewed questions: results/questions/REVIEW.md -> results/questions/frozen/.

The reviewer marks each question in REVIEW.md with exactly one of `[x] keep`, `[x] edit`,
`[x] drop`. For `edit`, the reviewer rewrites the question's heading text and/or its
`**Reference:**` line in the sheet; the frozen question takes the text as it stands there.
Evidence ranges are never edited in the sheet: they come from the draft. Any question left
unmarked, or marked twice, refuses the freeze. The frozen files and their SHA-256 are written
once; a second freeze refuses to overwrite them.
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
from datetime import UTC, datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
QDIR = HERE / "results" / "questions"
HEAD = re.compile(r"^### (\w+) \[(\w+)\] (.*)$")


def parse(sheet: str) -> dict[str, dict]:
    out, cur = {}, None
    for line in sheet.splitlines():
        m = HEAD.match(line)
        if m:
            cur = out[m.group(1)] = {"question": m.group(3).strip(), "marks": []}
        elif cur is not None and line.startswith("**Reference:**"):
            cur["reference_answer"] = line[len("**Reference:**"):].strip()
        elif cur is not None and line.startswith("- ["):
            cur["marks"] = re.findall(r"\[[xX]\] (keep|edit|drop)", line)
    return out


def freeze(sheet: str, drafts: list[dict]) -> tuple[list[dict], list[str]]:
    marks, frozen, errors = parse(sheet), [], []
    for d in drafts:
        kept = []
        for q in d["questions"]:
            m = marks.get(q["id"])
            if not m or len(m["marks"]) != 1:
                errors.append(f"{q['id']}: mark exactly one of keep/edit/drop")
                continue
            if m["marks"][0] == "drop":
                continue
            if not q["evidence"]:
                errors.append(f"{q['id']}: kept with no located evidence")
                continue
            item = {k: q[k] for k in ("id", "type", "question", "reference_answer", "evidence")}
            if m["marks"][0] == "edit":
                item.update(question=m["question"], reference_answer=m.get("reference_answer", q["reference_answer"]),
                            edited=True)
            kept.append(item)
        frozen.append({"document": d["document"], "sha256": d["sha256"], "model_ids": d["model_ids"],
                       "prompt_version": d["prompt_version"], "questions": kept})
    return frozen, errors


def main() -> int:
    out = QDIR / "frozen"
    if out.exists():
        print("already frozen; refusing to overwrite", file=sys.stderr)
        return 1
    drafts = [json.loads(p.read_text()) for p in sorted((QDIR / "draft").glob("*.json"))]
    frozen, errors = freeze((QDIR / "REVIEW.md").read_text(), drafts)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    out.mkdir(parents=True)
    manifest = {"frozen_at": datetime.now(UTC).isoformat(), "files": {}}
    for f in frozen:
        body = json.dumps(f, indent=1, ensure_ascii=False) + "\n"
        name = f"{Path(f['document']).stem}.json"
        (out / name).write_text(body)
        manifest["files"][name] = {"sha256": hashlib.sha256(body.encode()).hexdigest(), "questions": len(f["questions"])}
    (out / "MANIFEST.json").write_text(json.dumps(manifest, indent=1) + "\n")
    print(json.dumps(manifest, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
