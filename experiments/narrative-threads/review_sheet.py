"""Writes results/questions/REVIEW.md from the drafts: one entry per question with its
reference answer and the evidence as it stands in the document (bytes read back from the
corpus, never the model's quote), for the independent human review before freezing."""
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
DRAFTS = HERE / "results" / "questions" / "draft"
out = ["# Question review (phase 2)", "",
       "For each question mark: keep / edit / drop, and whether the evidence justifies the answer.",
       "Nothing is frozen until this review is done.", ""]
for p in sorted(DRAFTS.glob("*.json")):
    d = json.loads(p.read_text())
    data = (HERE / "corpus" / d["document"]).read_bytes()
    out += [f"## {d['document']} ({len(d['questions'])} questions, {d['rejected_quotes']} quotes not found)", ""]
    for q in d["questions"]:
        out += [f"### {q['id']} [{q['type']}] {q['question']}", "", f"**Reference:** {q['reference_answer']}", ""]
        for e in q["evidence"]:
            text = " ".join(data[e["start"]:e["end"]].decode("utf-8").split())
            out.append(f"- `{e['start']}:{e['end']}` {text}")
        out += ["", "- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer", ""]
(HERE / "results" / "questions" / "REVIEW.md").write_text("\n".join(out))
