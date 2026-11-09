"""One isolated `claude -p` call: the model sees only its prompt.

No settings, plugins or hooks (`--setting-sources ""`, `LOCKS_HOOKS_OFF=1`: a hook of the
person's tools would put its memory in front of the model), no tools, no MCP, run in an empty
temporary directory. Shared by every phase that calls a model.
"""
from __future__ import annotations

import json
import os
import subprocess
import tempfile
from typing import Any


def isolated_call(alias: str, schema: dict) -> tuple[list[str], dict[str, str]]:
    argv = ["claude", "-p", "--model", alias, "--output-format", "json", "--tools", "",
            "--strict-mcp-config", "--setting-sources", "", "--json-schema", json.dumps(schema)]
    return argv, {**os.environ, "LOCKS_HOOKS_OFF": "1"}


def ask(alias: str, schema: dict, prompt: str, timeout: int = 1800) -> dict[str, Any]:
    argv, env = isolated_call(alias, schema)
    with tempfile.TemporaryDirectory() as cwd:
        out = subprocess.run(argv, input=prompt, capture_output=True, text=True, timeout=timeout, env=env, cwd=cwd)
    return json.loads(out.stdout)


def answer_of(result: dict) -> dict:
    so = result.get("structured_output")
    if isinstance(so, dict):
        return so
    text = (result.get("result") or "").strip()
    if text.startswith("```"):
        text = text.split("\n", 1)[1].rsplit("```", 1)[0]
    return json.loads(text)
