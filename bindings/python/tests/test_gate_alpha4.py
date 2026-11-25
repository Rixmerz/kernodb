"""Regressions for the Python findings of the alpha.4 API gate."""

from __future__ import annotations

import time
from collections.abc import Iterator
from pathlib import Path

import kerno
import pytest


@pytest.fixture
def db(tmp_path: Path) -> Iterator[kerno.Database]:
    with kerno.open(tmp_path / "db") as database:
        yield database


@pytest.mark.parametrize("lifecycle", ["active", "archived", "deleted", "all"])
@pytest.mark.parametrize("bad", ["", "x" * 65])
def test_query_refuses_a_bad_unit_type_for_every_lifecycle(db: kerno.Database, lifecycle: str, bad: str) -> None:
    db.store("note", "a")
    with pytest.raises(kerno.InvalidArgumentError):
        db.query(bad, lifecycle=lifecycle)  # type: ignore[arg-type]


def test_argument_refusals_are_kerno_errors_and_value_errors(db: kerno.Database) -> None:
    calls = [
        lambda: db.read("zz"),
        lambda: db.related("00" * 16, "sideways"),  # type: ignore[arg-type]
        lambda: db.query(lifecycle="nope"),  # type: ignore[arg-type]
        lambda: db.history("00" * 16, order="up"),  # type: ignore[arg-type]
        lambda: db.hybrid_search(text="x", metric="l2"),
        lambda: db.run_retention(default={"keep_last": -1}),
        lambda: db.commit([("nope",)]),
        lambda: db.define_schema("u", [{"name": "a"}]),
    ]
    for call in calls:
        with pytest.raises(kerno.InvalidArgumentError) as info:
            call()
        assert isinstance(info.value, kerno.KernoError)
        assert isinstance(info.value, ValueError)


def test_a_closed_handle_raises_closed_error(tmp_path: Path) -> None:
    db = kerno.open(tmp_path / "db")
    uid = db.store("note", "a")
    db.close()
    with pytest.raises(kerno.ClosedError, match="closed") as info:
        db.read(uid)
    assert isinstance(info.value, kerno.KernoError)
    assert isinstance(info.value, ValueError)
    assert not isinstance(info.value, kerno.InvalidArgumentError)


def _compacted_deleted_unit(db: kerno.Database) -> tuple[str, int]:
    uid = db.store("note", "v1")
    db.update(uid, "v2")
    db.delete(uid)
    seq = db.last_commit_seq()
    db.compact()
    return uid, seq


def test_history_unavailable_has_its_own_class_and_a_plain_message(db: kerno.Database) -> None:
    uid, seq = _compacted_deleted_unit(db)
    for call in (lambda: db.read_as_of(uid, 2), lambda: db.compare(uid, 1, 2)):
        with pytest.raises(kerno.HistoryUnavailableError) as info:
            call()
        assert isinstance(info.value, kerno.KernoError)
        assert not isinstance(info.value, ValueError)
        assert "Audit" not in str(info.value)
        assert "policy" not in str(info.value)


def test_history_with_payload_lists_the_revisions_of_a_compacted_unit(db: kerno.Database) -> None:
    uid, _ = _compacted_deleted_unit(db)
    revisions = db.history(uid, with_payload=True, order="asc")
    assert [r.revision for r in revisions] == [1, 2, 3]
    assert all(r.payload is None for r in revisions)


def test_revision_and_unit_timestamps_are_microseconds(db: kerno.Database) -> None:
    before = int(time.time() * 1e6)
    uid = db.store("note", "a")
    db.update(uid, "b")
    after = int(time.time() * 1e6)
    unit = db.read(uid)
    assert unit is not None
    assert before - 1_000_000 <= unit.created_at_us <= unit.updated_at_us <= after + 1_000_000
    rev = db.history(uid)[0]
    assert before - 1_000_000 <= rev.created_at_us <= after + 1_000_000
    assert not hasattr(rev, "created_at")


def test_read_document_of_a_payload_that_is_not_a_document_is_a_schema_error(db: kerno.Database) -> None:
    uid = db.store("note", "plain text")
    with pytest.raises(kerno.SchemaError):
        db.read_document(uid)


def test_field_type_str_is_refused(db: kerno.Database) -> None:
    with pytest.raises(kerno.InvalidArgumentError, match="unknown field type"):
        db.define_schema("u", [{"name": "a", "type": "str"}])


def test_upsert_by_a_non_unique_field_raises_not_unique(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "age", "type": "int"}])
    with pytest.raises(kerno.NotUniqueError):
        db.upsert("user", "age", {"age": 1})
    assert "NotUniqueError" in (kerno.Database.upsert.__doc__ or "")


def test_max_batch_ops_messages_name_the_argument_and_the_range(tmp_path: Path) -> None:
    for bad in (0, 2**20 + 1, -1):
        with pytest.raises(kerno.InvalidArgumentError, match=r"max_batch_ops must be .*from 1 to 1048576") as info:
            kerno.open(tmp_path / "x", max_batch_ops=bad)
        assert "max_changeset_ops" not in str(info.value)


def test_a_limit_below_one_names_the_range(db: kerno.Database) -> None:
    for call in (lambda: db.search_text("x", limit=0), lambda: db.history("00" * 16, limit=0)):
        with pytest.raises(kerno.InvalidArgumentError, match=r"limit must be a whole number from 1 to"):
            call()


def test_a_wrong_typed_list_argument_is_readable(db: kerno.Database) -> None:
    for call in (
        lambda: db.search_text("x", types="note"),  # type: ignore[arg-type]
        lambda: db.search_substring("x", types="note"),  # type: ignore[arg-type]
        lambda: db.hybrid_search(text="x", text_types="note"),  # type: ignore[arg-type]
        lambda: db.hybrid_search(seeds="00" * 16),  # type: ignore[arg-type]
        lambda: db.search_text("x", types=[1]),  # type: ignore[list-item]
    ):
        with pytest.raises(kerno.InvalidArgumentError, match="must be a list of strings") as info:
            call()
        assert "extract" not in str(info.value)


def test_refresh_on_a_read_write_handle_has_no_rust_type_names(db: kerno.Database) -> None:
    with pytest.raises(kerno.InvalidArgumentError, match="read_only=True") as info:
        db.refresh()
    assert "Config" not in str(info.value) and "AccessMode" not in str(info.value)


def test_the_package_has_a_version() -> None:
    assert isinstance(kerno.__version__, str) and kerno.__version__
    assert "__version__" in kerno.__all__
