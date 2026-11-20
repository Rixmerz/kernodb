from freeze import parse
from review_llm import apply

SHEET = ("### x01 [local] Q1?\n\n**Reference:** A1\n\n- b\n\n- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer\n\n"
         "### x02 [local] Q2?\n\n**Reference:** A2\n\n- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer\n")


def test_apply_marks_and_edits_are_read_by_freeze():
    out = apply(SHEET, {
        "x01": {"decision": "keep", "evidence_justifies": True, "reason": "ok", "reviewer": "m"},
        "x02": {"decision": "edit", "question": "Q2 new?", "reference_answer": "A2 new", "evidence_justifies": True,
                "reason": "r", "reviewer": "m"}})
    p = parse(out)
    assert p["x01"]["marks"] == ["keep"]
    assert p["x02"]["marks"] == ["edit"] and p["x02"]["question"] == "Q2 new?" and p["x02"]["reference_answer"] == "A2 new"
