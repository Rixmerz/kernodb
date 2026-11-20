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


def test_update_keeps_history_and_as_of(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a = db.store("note", "v1")
        db.update(a, "v2")
        hist = db.history(a, with_payload=True)
        assert [r.kind for r in hist] == ["update", "create"]
        assert [r.payload for r in hist] == [b"v2", b"v1"]
        assert db.read(a).revision == 2
        old = db.read_as_of(a, hist[1].commit_seq)
        assert old.payload == b"v1"
        assert db.read_as_of(a, 0) is None
        assert hist[0].content_hash != hist[1].content_hash


def test_commit_batch_and_delete(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a = db.store("note", "a")
        b = db.store("note", "b")
        rel = db.relate(a, b, "next")
        new = db.commit([("store", "note", b"c"), ("update", a, "a2"), ("unrelate", rel), ("delete", b)])
        assert len(new) == 1 and db.read(new[0]).payload == b"c"
        assert db.read(a).payload == b"a2"
        assert db.read(b).lifecycle == "deleted"
        assert db.related(a) == []
        assert db.history(b)[0].kind == "delete"
        with pytest.raises(ValueError):
            db.commit([("bogus",)])


def test_verify_reports_clean(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a = db.store("note", "x")
        db.update(a, "y")
        report = db.verify()
        assert report["clean"] is True
        assert report["units"] == 1 and report["revisions"] == 2


def test_relations_inside_a_batch_and_detailed_commit(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a = db.store("n", "a")
        out = db.commit(
            [("store", "n", "b"), ("store", "n", "c"), ("relate", 0, 1, "next"), ("relate", a, 0, "next")],
            detailed=True,
        )
        assert len(out["units"]) == 2 and len(out["relations"]) == 2
        assert out["commit_seq"] == db.last_commit_seq()
        assert db.related(a)[0].to_id == out["units"][0]
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("relate", a, 7, "next")])


def test_unique_relate_and_cascade_delete(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a, b = db.store("n", "a"), db.store("n", "b")
        r1 = db.relate(a, b, "r", unique=True)
        assert db.relate(a, b, "r", unique=True) == r1
        db.relate(b, a, "r")
        with pytest.raises(kerno.HasRelationsError):
            db.delete(a)
        db.commit([("delete_cascade", a), ("delete_cascade", b)])
        assert db.read(a).lifecycle == "deleted" and db.related(b) == []


def test_update_if_changed_and_restore_with_provenance(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a = db.store("n", "v1")
        assert db.update(a, "v1", if_changed=True) is False
        assert db.read(a).revision == 1
        assert db.update(a, "v2", if_changed=True) is True
        db.commit([("restore", a, 1)], reason="rollback", actor="tester")
        assert db.read(a).payload == b"v1"
        db.commit([("archive", a)])
        assert db.read(a).lifecycle == "archived"
        db.commit([("unarchive", a)])
        assert db.read(a).lifecycle == "active"


def test_missing_ids_and_future_sequences_do_not_raise(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a = db.store("n", "v1")
        assert db.history("0" * 32) == []
        assert db.read_as_of("0" * 32, 1) is None
        assert db.read_as_of(a, 10**12).payload == b"v1"
        with pytest.raises(kerno.NotFoundError):
            db.update("0" * 32, "x")
        with pytest.raises(kerno.PayloadTooLargeError):
            db.store("n", b"x" * (2 << 20))
        assert issubclass(kerno.NotFoundError, kerno.KernoError)


def test_documents_schema_index_find_and_execute(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema(
            "sym",
            [{"name": "qname", "type": "text", "required": True, "unique": True}, {"name": "lines", "type": "int"}],
        )
        a = db.store_document("sym", {"qname": "m.f", "lines": 3})
        db.store_document("sym", {"qname": "m.g", "lines": 9})
        assert [u.id for u in db.find("sym", "qname", "m.f")] == [a]
        assert db.read_document(a) == {"qname": "m.f", "lines": 3}
        with pytest.raises(kerno.SchemaError):
            db.store_document("sym", {"qname": "m.f"})  # unique
        with pytest.raises(kerno.SchemaError):
            db.update(a, b"not a document")
        res = db.execute("FIND sym AS s WHERE s.lines > 5")
        assert res["aliases"] == ["s"] and len(res["rows"]) == 1


def test_locked_and_read_only(tmp_path):
    path = tmp_path / "db"
    with kerno.open(path) as db:
        db.store("n", "a")
        with pytest.raises(kerno.LockedError):
            kerno.open(path)
    with kerno.open(path, read_only=True) as ro:
        assert len(ro.query("n")) == 1
        with pytest.raises(kerno.KernoError):
            ro.store("n", "b")


def test_verify_reports_storage_corruption(tmp_path):
    path = tmp_path / "db"
    with kerno.open(path) as db:
        db.commit([("store", "c", "payload-%d" % i * 20) for i in range(2000)])
    sst = max(path.glob("*.sst"), key=lambda f: f.stat().st_size)
    data = bytearray(sst.read_bytes())
    data[len(data) // 2] ^= 0xFF
    sst.write_bytes(bytes(data))
    with kerno.open(path) as db:
        report = db.verify()
        assert report["clean"] is False and "corruption" in report["storage_error"]
        with pytest.raises(kerno.CorruptionError):
            db.query("c")


def test_relations_bulk_read(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        a, b, c = (db.store("n", x) for x in "abc")
        db.commit([("relate", a, b, "calls"), ("relate", b, c, "calls"), ("relate", a, c, "other")])
        assert len(db.relations()) == 3
        assert {(r.from_id, r.to_id) for r in db.relations("calls")} == {(a, b), (b, c)}


def test_max_batch_ops(tmp_path):
    with kerno.open(tmp_path / "a") as db:
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("store", "n", b"\xff")] * 10_001)
    with kerno.open(tmp_path / "b", max_batch_ops=60_000) as db:
        out = db.commit([("store", "n", b"\xff")] * 30_000 + [("relate", 0, 1, "r")] * 20_000, detailed=True)
        assert len(out["units"]) == 30_000 and len(out["relations"]) == 20_000


def test_set_fulltext(tmp_path):
    # The keyword index is off by default and turned on per type (ADR-0023); a type that
    # already has units is reindexed (or cleared) in the same commit.
    with kerno.open(tmp_path / "db") as db:
        db.store("sym", '{"qname": "m.f"}')
        db.store("note", "hello")
        db.set_fulltext("note", True)
        db.set_fulltext("note", True)  # same state: nothing written
        assert db.verify()["clean"] is True
        db.set_fulltext("note", False)
        assert db.verify()["clean"] is True
