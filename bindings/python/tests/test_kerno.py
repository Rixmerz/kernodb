from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import kerno
import pytest


@pytest.fixture
def db(tmp_path: Path) -> Iterator[kerno.Database]:
    with kerno.open(tmp_path / "db") as database:
        yield database


def test_store_and_read_round_trip(db: kerno.Database) -> None:
    uid = db.store("note", "Rust has no GC")
    unit = db.read(uid)
    assert unit is not None
    assert unit.id == uid
    assert unit.unit_type == "note"
    assert unit.payload == b"Rust has no GC"
    assert unit.lifecycle == "active"
    assert unit.schema_version == 1


def test_bytes_payload_is_kept_verbatim(db: kerno.Database) -> None:
    raw = bytes(range(256))
    assert db.read(db.store("blob", raw)).payload == raw  # type: ignore[union-attr]


def test_read_unknown_id_is_none(db: kerno.Database) -> None:
    assert db.read("00" * 16) is None


def test_relate_and_related_in_both_directions(db: kerno.Database) -> None:
    note, topic = db.store("note", "a"), db.store("topic", "b")
    rel = db.relate(note, topic, "about")
    (out,) = db.related(note)
    assert (out.id, out.from_id, out.to_id, out.relation_type) == (rel, note, topic, "about")
    (inbound,) = db.related(topic, direction="in")
    assert inbound.id == rel
    assert db.related(note, relation_type="other") == []
    assert db.related(topic) == []


def test_query_filters_by_type(db: kerno.Database) -> None:
    n1, n2 = db.store("note", "1"), db.store("note", "2")
    db.store("topic", "t")
    assert len(db.query()) == 3
    assert sorted(u.id for u in db.query("note")) == sorted([n1, n2])
    assert db.query("missing") == []
    assert db.query("note", schema_version=2) == []


def test_data_survives_close_and_reopen(tmp_path: Path) -> None:
    path = tmp_path / "db"
    database = kerno.open(path)
    uid = database.store("note", "durable")
    database.close()
    with kerno.open(str(path)) as again:  # a str path works as well as a Path
        assert again.read(uid).payload == b"durable"  # type: ignore[union-attr]


def test_engine_errors_raise_kerno_error(db: kerno.Database) -> None:
    with pytest.raises(kerno.KernoError):
        db.store("", "empty type is invalid")
    ghost = "ab" * 16
    with pytest.raises(kerno.KernoError):
        db.relate(ghost, ghost, "about")


@pytest.mark.parametrize("bad", ["", "zz" * 16, "ab" * 15, "é" * 16])
def test_malformed_id_is_value_error(db: kerno.Database, bad: str) -> None:
    with pytest.raises(ValueError, match="32 hex digits"):
        db.read(bad)


def test_bad_direction_is_value_error(db: kerno.Database) -> None:
    with pytest.raises(ValueError, match="direction"):
        db.related("00" * 16, direction="sideways")  # type: ignore[arg-type]


def test_payload_must_be_str_or_bytes(db: kerno.Database) -> None:
    with pytest.raises(TypeError):
        db.store("note", 42)  # type: ignore[arg-type]


def test_closed_database_refuses_work_and_closes_twice(tmp_path: Path) -> None:
    database = kerno.open(tmp_path / "db")
    database.close()
    database.close()
    with pytest.raises(ValueError, match="closed"):
        database.store("note", "x")


def test_second_open_of_a_held_directory_fails(tmp_path: Path) -> None:
    with kerno.open(tmp_path / "db"):
        with pytest.raises(kerno.KernoError):
            kerno.open(tmp_path / "db")


def test_stubs_ship_with_the_package() -> None:
    package = Path(kerno.__file__).parent
    assert (package / "py.typed").exists()
    assert (package / "__init__.pyi").exists()
