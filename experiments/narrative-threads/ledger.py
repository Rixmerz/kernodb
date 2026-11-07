"""Cost ledger: every model call of the experiment, and the hard caps.

Later phases call `claude -p --model <alias> --output-format json`. Before each call,
`Ledger.authorize(phase)` refuses it once its phase cap or the total cap is reached;
after it, `Ledger.record(...)` appends one JSON line with the CLI's figures and locks'
reading of the same session's transcript. The larger of the two dollar figures is what
counts against the caps; when locks cannot read the transcript its figure is recorded as
unknown (null) and the CLI's is used, never $0.

The ledger only reads locks (`locks.usage`, in a separate Python process): it never opens
or writes locks' store.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from collections.abc import Callable
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

CAPS_USD = {"extraction": 15.0, "queries": 7.0, "eval": 3.0}
TOTAL_CAP_USD = 25.0
DEFAULT_PATH = Path(__file__).resolve().parent / "results" / "ledger.jsonl"


class CapReached(RuntimeError):
    pass


def locks_src() -> Path | None:
    """locks' `src/`: `NT_LOCKS_SRC`, else the newest install in Claude Code's plugin cache."""
    if os.environ.get("NT_LOCKS_SRC"):
        return Path(os.environ["NT_LOCKS_SRC"])
    root = Path(os.environ.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude") / "plugins" / "cache"
    found = [p for p in root.glob("*/locks/*/src") if (p / "locks" / "usage.py").exists()]

    def key(p: Path):
        return tuple(int(x) if x.isdigit() else 0 for x in p.parent.name.split("."))
    return max(found, key=key) if found else None


_PROBE = """
import json, sys
sys.path.insert(0, sys.argv[1])
from locks import usage
path = usage.transcript_of(sys.argv[2])
print(json.dumps(None if path is None else {
    "path": str(path), "cost": usage.transcript_cost(path), "turns": usage.transcript_turns(path)}))
"""


def locks_reading(session_id: str | None) -> dict | None:
    """locks' figures for one session's transcript, or None when they cannot be read."""
    src = locks_src()
    if not session_id or src is None:
        return None
    try:
        out = subprocess.run([sys.executable, "-c", _PROBE, str(src), session_id],
                             capture_output=True, text=True, timeout=60, check=True)
        return json.loads(out.stdout.strip() or "null")
    except (subprocess.SubprocessError, OSError, ValueError):
        return None


def parse_cli(result: dict[str, Any]) -> dict[str, Any]:
    """The fields the ledger keeps from one `claude -p --output-format json` result."""
    models = {}
    for model_id, u in (result.get("modelUsage") or {}).items():
        models[model_id] = {k: int(u.get(k) or 0) for k in
                            ("inputTokens", "outputTokens", "cacheReadInputTokens", "cacheCreationInputTokens")}
    cost = result.get("total_cost_usd")
    return {"session_id": result.get("session_id"),
            "cli_cost_usd": float(cost) if isinstance(cost, (int, float)) and not isinstance(cost, bool) else None,
            "models": models, "is_error": bool(result.get("is_error"))}


class Ledger:
    def __init__(self, path: Path = DEFAULT_PATH,
                 reader: Callable[[str | None], dict | None] = locks_reading) -> None:
        self.path = Path(path)
        self.reader = reader

    def entries(self) -> list[dict]:
        if not self.path.exists():
            return []
        return [json.loads(ln) for ln in self.path.read_text().splitlines() if ln.strip()]

    def spent(self, phase: str | None = None) -> float:
        return round(sum(e["charged_usd"] for e in self.entries() if phase is None or e["phase"] == phase), 6)

    def authorize(self, phase: str) -> None:
        """Raise CapReached when a call of this phase may not start."""
        if phase not in CAPS_USD:
            raise ValueError(f"unknown phase {phase!r}; one of {sorted(CAPS_USD)}")
        if self.spent(phase) >= CAPS_USD[phase]:
            raise CapReached(f"{phase} cap of USD {CAPS_USD[phase]} reached ({self.spent(phase):.4f} spent)")
        if self.spent() >= TOTAL_CAP_USD:
            raise CapReached(f"total cap of USD {TOTAL_CAP_USD} reached ({self.spent():.4f} spent)")

    def record(self, phase: str, document: str, alias: str, result: dict[str, Any],
               now: datetime | None = None) -> dict:
        if phase not in CAPS_USD:
            raise ValueError(f"unknown phase {phase!r}")
        cli = parse_cli(result)
        locks = self.reader(cli["session_id"])
        locks_cost = locks.get("cost") if locks else None
        figures = [x for x in (cli["cli_cost_usd"], locks_cost) if x is not None]
        if not figures:
            raise ValueError("neither the CLI nor locks reported a cost; refusing to record $0")
        entry = {"at": (now or datetime.now(UTC)).isoformat(), "phase": phase, "document": document,
                 "alias": alias, "model_ids": sorted(cli["models"]), **cli,
                 "locks_cost_usd": locks_cost, "locks_turns": locks.get("turns") if locks else None,
                 "locks_transcript": locks.get("path") if locks else None,
                 "charged_usd": max(figures)}
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.path.open("a") as f:
            f.write(json.dumps(entry, sort_keys=True) + "\n")
        return entry
