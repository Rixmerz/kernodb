from evaluate import score
from freeze import freeze

DRAFT = {"document": "x.txt", "sha256": "h", "model_ids": ["m"], "prompt_version": "q3", "questions": [
    {"id": "x01", "type": "local", "question": "Q1?", "reference_answer": "A1", "evidence": [{"start": 0, "end": 5}]},
    {"id": "x02", "type": "local", "question": "Q2?", "reference_answer": "A2", "evidence": [{"start": 9, "end": 12}]},
    {"id": "x03", "type": "local", "question": "Q3?", "reference_answer": "A3", "evidence": []}]}


def sheet(m1, m2, m3, q2="Q2?"):
    return (f"### x01 [local] Q1?\n\n**Reference:** A1\n\n- [{m1[0]}] keep  - [{m1[1]}] edit  - [{m1[2]}] drop\n"
            f"### x02 [local] {q2}\n\n**Reference:** A2 better\n\n- [{m2[0]}] keep  - [{m2[1]}] edit  - [{m2[2]}] drop\n"
            f"### x03 [local] Q3?\n\n**Reference:** A3\n\n- [{m3[0]}] keep  - [{m3[1]}] edit  - [{m3[2]}] drop\n")


def test_freeze_applies_marks_and_edits():
    frozen, errors = freeze(sheet("x  ", " x ", "  x", "Q2 edited?"), [DRAFT])
    assert not errors
    qs = frozen[0]["questions"]
    assert [q["id"] for q in qs] == ["x01", "x02"]
    assert qs[1]["question"] == "Q2 edited?" and qs[1]["reference_answer"] == "A2 better" and qs[1]["edited"]


def test_freeze_refuses_unmarked_double_or_evidence_less():
    _, errors = freeze(sheet("   ", "xx ", "x  "), [DRAFT])
    assert len(errors) == 3


def test_score():
    m = score([{"start": 0, "end": 5}, {"start": 50, "end": 60}], [(3, 10), (20, 30)])
    assert m == {"coverage": 0.5, "precision": 0.5}
