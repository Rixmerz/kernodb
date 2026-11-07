import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

import ledger as L


_n = iter(range(10**6))


def cli(cost, sid=None):
    sid = sid or f"session-{next(_n)}"
    return {"type": "result", "result": "ok", "session_id": sid, "total_cost_usd": cost,
            "modelUsage": {"claude-sonnet-4-5-20250929": {"inputTokens": 10, "outputTokens": 20,
                                                          "cacheReadInputTokens": 30, "cacheCreationInputTokens": 40,
                                                          "costUSD": cost}}}


def test_records_both_figures_and_charges_the_larger(tmp_path):
    led = L.Ledger(tmp_path / "l.jsonl", reader=lambda sid: {"cost": 2.5, "turns": 3, "path": "/x"})
    e = led.record("extraction", "a", "sonnet", cli(1.0), call_id="c1")
    assert e["cli_cost_usd"] == 1.0 and e["locks_cost_usd"] == 2.5 and e["charged_usd"] == 2.5
    assert e["model_ids"] == ["claude-sonnet-4-5-20250929"]
    assert e["models"]["claude-sonnet-4-5-20250929"]["cacheCreationInputTokens"] == 40
    assert json.loads((tmp_path / "l.jsonl").read_text())["document"] == "a"


def test_unknown_locks_cost_is_null_not_zero(tmp_path):
    led = L.Ledger(tmp_path / "l.jsonl", reader=lambda sid: None)
    e = led.record("queries", "b", "haiku", cli(0.25), call_id="c2")
    assert e["locks_cost_usd"] is None and e["charged_usd"] == 0.25


def test_no_cost_at_all_is_refused(tmp_path):
    led = L.Ledger(tmp_path / "l.jsonl", reader=lambda sid: None)
    with pytest.raises(ValueError):
        led.record("queries", "b", "haiku", cli(None), call_id="c3")


def test_phase_caps_refuse_the_next_call(tmp_path):
    led = L.Ledger(tmp_path / "l.jsonl", reader=lambda sid: None)
    led.authorize("eval")
    led.record("eval", "c", "haiku", cli(3.0), call_id="c4")
    with pytest.raises(L.CapReached):
        led.authorize("eval")
    led.authorize("queries")  # other phases still open
    with pytest.raises(ValueError):
        led.authorize("nonsense")


def test_total_cap(tmp_path):
    led = L.Ledger(tmp_path / "l.jsonl", reader=lambda sid: None)
    led.record("extraction", "a", "sonnet", cli(14.0), call_id="c5")
    led.record("queries", "a", "haiku", cli(6.0), call_id="c6")
    led.record("eval", "a", "haiku", cli(2.0), call_id="c7")
    led.authorize("eval")  # 22 < 25
    led.record("extraction", "a", "sonnet", cli(3.0), call_id="c8")  # a call may overshoot; the next is refused
    for phase in L.CAPS_USD:
        with pytest.raises(L.CapReached):
            led.authorize(phase)


def test_locks_reads_a_fake_transcript(tmp_path, monkeypatch):
    src = L.locks_src()
    if src is None:
        pytest.skip("locks is not installed")
    sid = "fake-session-0001"
    folder = tmp_path / "cfg" / "projects" / "-tmp-x"
    folder.mkdir(parents=True)
    rows = [{"type": "assistant", "timestamp": "2026-11-07T10:00:00Z",
             "message": {"id": "m1", "model": "claude-sonnet-4-5",
                         "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}},
            {"type": "assistant", "timestamp": "2026-11-07T10:00:01Z",  # same message again: once
             "message": {"id": "m1", "model": "claude-sonnet-4-5",
                         "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}}]
    (folder / f"{sid}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
    monkeypatch.setenv("NT_LOCKS_SRC", str(src))
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path / "cfg"))
    expected = json.loads(subprocess.run(
        [sys.executable, "-c", f"import sys;sys.path.insert(0,{str(src)!r});from locks import usage;"
         "print(usage.dollars('claude-sonnet-4-5',1000000,0))"], capture_output=True, text=True, check=True).stdout)
    reading = L.locks_reading(sid)
    assert reading["cost"] == pytest.approx(expected) and reading["cost"] > 0 and reading["turns"] == 1
    assert L.locks_reading("missing-session") is None
    e = L.Ledger(tmp_path / "l.jsonl").record("extraction", "a", "sonnet", cli(0.01, sid), call_id="c9")
    assert e["charged_usd"] == pytest.approx(expected) and e["locks_transcript"].endswith(f"{sid}.jsonl")


def test_same_call_or_session_is_never_recorded_twice(tmp_path):
    led = L.Ledger(tmp_path / "l.jsonl", reader=lambda sid: None)
    led.record("extraction", "a", "sonnet", cli(1.0, "s-1"), call_id="x1")
    with pytest.raises(L.DuplicateCall):
        led.record("extraction", "a", "sonnet", cli(1.0, "s-1"), call_id="x2")
    with pytest.raises(L.DuplicateCall):
        led.record("extraction", "a", "sonnet", cli(1.0, "s-2"), call_id="x1")
    with pytest.raises(ValueError):
        led.record("extraction", "a", "sonnet", {"total_cost_usd": 1.0}, call_id="x3")
    assert len(led.entries()) == 1 and led.spent() == 1.0


def test_keeps_both_figures_and_no_content(tmp_path):
    led = L.Ledger(tmp_path / "l.jsonl", reader=lambda sid: {"cost": 0.4, "turns": 1, "path": "/t.jsonl"})
    r = cli(0.5, "s-9")
    r["result"] = "SECRET-ANSWER-TEXT"
    r["prompt"] = "SECRET-PROMPT-TEXT"
    e = led.record("queries", "a", "haiku", r, call_id="q1")
    assert (e["cli_cost_usd"], e["locks_cost_usd"], e["charged_usd"]) == (0.5, 0.4, 0.5)
    raw = (tmp_path / "l.jsonl").read_text()
    assert "SECRET" not in raw
    assert set(e) <= {"at", "call_id", "phase", "document", "alias", "model_ids", "session_id", "cli_cost_usd",
                      "models", "is_error", "locks_cost_usd", "locks_turns", "locks_transcript", "charged_usd"}
