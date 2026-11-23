"""The reopen row of bench/vs_sqlite.py must do the same work on both engines.

Run from bindings/python: `.venv/bin/pytest benchmarks` (the default testpaths do not include it).
"""

import sys
from pathlib import Path

import kerno

sys.path.insert(0, str(Path(__file__).parent.parent / "bench"))
import vs_sqlite  # noqa: E402


class Handle:
    def __init__(self, log, name):
        self.log, self.name = log, name

    def close(self):
        self.log.append((self.name, "close"))


def test_reopen_times_runs_open_then_close_on_each_side_once(monkeypatch):
    log = []

    def fake_open(name):
        return lambda path: (log.append((name, "open", path)), Handle(log, name))[1]

    monkeypatch.setattr(vs_sqlite, "open_sqlite", fake_open("sqlite"))
    vs_sqlite.reopen_times(fake_open("kerno"), "kdir", "sfile")
    assert log == [("kerno", "open", "kdir"), ("kerno", "close"), ("sqlite", "open", "sfile"), ("sqlite", "close")]


def test_open_sqlite_reads_the_file_and_keeps_wal_and_full_sync(tmp_path):
    path = tmp_path / "s.db"
    setup = vs_sqlite.sqlite3.connect(path, isolation_level=None)
    setup.execute("PRAGMA journal_mode=WAL")
    setup.execute("create table t(x)")
    setup.close()
    conn = vs_sqlite.open_sqlite(path)
    try:
        assert conn.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
        assert conn.execute("PRAGMA synchronous").fetchone()[0] == 2  # FULL
        assert conn.isolation_level is None
    finally:
        conn.close()


def test_reopen_times_on_real_stores_returns_two_positive_times(tmp_path):
    kdir = tmp_path / "k"
    kerno.open(kdir).close()
    path = tmp_path / "s.db"
    conn = vs_sqlite.sqlite3.connect(path, isolation_level=None)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.close()
    k, q = vs_sqlite.reopen_times(kerno.open, kdir, path)
    assert k > 0 and q > 0
