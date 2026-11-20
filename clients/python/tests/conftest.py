"""Starts the real kerno-server binary for the tests.

The binary is ``$KERNO_SERVER_BIN`` when that is set, else the one ``cargo build -p kerno-server``
makes. A missing binary fails the tests: a client tested against nothing proves nothing.
"""

from __future__ import annotations

import json
import os
import select
import signal
import subprocess
from pathlib import Path
from typing import Iterator, NamedTuple

import pytest
from kerno_client import KernoClient

REPO = Path(__file__).resolve().parents[3]


def _build_binary() -> str:
    build = subprocess.run(
        ["cargo", "build", "--quiet", "-p", "kerno-server", "--message-format=json"],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    assert build.returncode == 0, f"cargo build -p kerno-server failed:\n{build.stderr}"
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == "kerno-server":
            if message.get("executable"):
                return message["executable"]
    raise AssertionError("cargo did not report the kerno-server executable")


@pytest.fixture(scope="session")
def server_binary() -> str:
    return os.environ.get("KERNO_SERVER_BIN") or _build_binary()


class Running(NamedTuple):
    url: str
    process: subprocess.Popen
    db: Path
    log: Path


def start_server(binary: str, db: Path, log: Path) -> Running:
    stderr = open(log, "ab")
    process = subprocess.Popen(
        [binary, "--db", str(db), "--listen", "127.0.0.1:0"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=stderr,
        text=True,
    )
    assert process.stdout is not None
    ready, _, _ = select.select([process.stdout], [], [], 30)
    line = process.stdout.readline() if ready else ""
    prefix = "listening on "
    assert line.startswith(prefix), f"the server did not start: {line!r}\n{log.read_text()}"
    return Running(line[len(prefix):].strip(), process, db, log)


def stop_server(running: Running) -> int:
    running.process.send_signal(signal.SIGTERM)
    return running.process.wait(timeout=30)


@pytest.fixture
def server(server_binary: str, tmp_path: Path) -> Iterator[Running]:
    # Two levels that do not exist yet: the server must create both.
    running = start_server(server_binary, tmp_path / "data" / "db", tmp_path / "stderr.txt")
    yield running
    if running.process.poll() is None:
        running.process.kill()
        running.process.wait()


@pytest.fixture
def kerno(server: Running) -> KernoClient:
    return KernoClient(server.url)
