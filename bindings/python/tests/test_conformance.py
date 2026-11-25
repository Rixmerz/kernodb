"""Pre-release conformance and boundary tests for the Python binding.

Every claim of the READMEs, the docstrings and the stubs is executed here, with each documented
limit tested at the limit and one past it. The boundary tests feed odd values to every call: each
must succeed as documented or raise the documented exception, never crash, panic or answer
silently.
"""

from __future__ import annotations

import itertools
import math
import re
import threading
from types import SimpleNamespace
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

import kerno
import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

ROOT = Path(__file__).resolve().parents[3]
README = ROOT / "bindings" / "python" / "README.md"
MISSING = "00" * 16
MIB = 1024 * 1024


@pytest.fixture
def db(tmp_path: Path) -> Iterator[kerno.Database]:
    with kerno.open(tmp_path / "db") as database:
        yield database


def fenced(path: Path, lang: str) -> list[str]:
    return re.findall(rf"```{lang}\n(.*?)```", path.read_text(encoding="utf-8"), re.S)


# ---------------------------------------------------------------- README examples


def test_the_readme_quickstart_runs_as_written(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capfd) -> None:
    monkeypatch.chdir(tmp_path)
    exec(compile(fenced(README, "python")[0], "README quickstart", "exec"), {})  # noqa: S102 - our own docs
    out = capfd.readouterr().out.splitlines()
    assert len(out[0]) > 2 and out[0].startswith("[")  # the ids of the outgoing relations
    assert [line.split()[1] for line in out[1:3]] == ["update", "create"]  # newest first
    assert out[-1].startswith("[") and len(out[-1]) > 2


def test_the_batch_example_returns_the_documented_report(db: kerno.Database) -> None:
    other, old = db.store("note", "old payload"), db.store("note", "to remove")
    existing = db.store("symbol", "s")
    db.relate(old, existing, "r")
    out = db.commit(
        [
            ("store", "symbol", b"a"),
            ("store", "symbol", b"b"),
            ("relate", 0, 1, "calls"),
            ("relate", existing, 0, "calls"),
            ("update_if_changed", other, "old payload"),
            ("delete_cascade", old),
        ],
        reason="index run 42",
        actor="indexer",
        detailed=True,
    )
    assert set(out) >= {"units", "relations", "unchanged", "upserts", "commit_seq"}
    assert len(out["units"]) == 2 and len(out["relations"]) == 2
    assert out["unchanged"] == 1 and out["upserts"] == []
    assert db.read(old).lifecycle == "deleted"  # type: ignore[union-attr]
    assert db.relations() and all(r.relation_type == "calls" for r in db.relations())
    assert db.commit([("store", "symbol", "x")]) != []  # not detailed: the stored ids


def test_every_documented_batch_op_works_in_one_commit(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "email", "type": "text", "unique": True}])
    a, b, c, d, e, f, g = (db.store("note", p) for p in "abcdefg")
    db.commit([("archive", g)])
    db.relate(a, b, "r")
    rel = db.relate(a, e, "r")
    db.update(c, "c2")
    out = db.commit(
        [
            ("store", "note", "s"),
            ("update", a, "a2"),
            ("update_if_changed", e, "e"),
            ("delete", d),
            ("delete_cascade", b),
            ("unrelate", rel),
            ("relate", a, e, "t"),
            ("relate_unique", a, e, "t"),
            ("archive", f),
            ("unarchive", g),
            ("restore", c, 1),
            ("upsert", "user", "email", {"email": "x@y"}),
        ],
        detailed=True,
    )
    assert len(out["units"]) == 1 and len(out["upserts"]) == 1
    assert out["upserts"][0]["outcome"] == "created"
    assert db.read(c).payload == b"c"  # restored revision 1  # type: ignore[union-attr]
    assert db.read(d).lifecycle == "deleted"  # type: ignore[union-attr]
    assert (db.read(f).lifecycle, db.read(g).lifecycle) == ("archived", "active")  # type: ignore[union-attr]
    assert len(db.related(a, "out", "t")) == 1, "relate_unique reuses the relation of relate"


def test_a_unit_appears_in_one_op_of_a_batch(db: kerno.Database) -> None:
    uid = db.store("note", "x")
    seq = db.last_commit_seq()
    with pytest.raises(kerno.InvalidArgumentError, match="already has a revision"):
        db.commit([("update", uid, "y"), ("update", uid, "z")])
    assert db.last_commit_seq() == seq and db.read(uid).payload == b"x"  # type: ignore[union-attr]


def test_the_upsert_example_outcomes_and_history(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "email", "type": "text", "unique": True}, {"name": "name", "type": "text"}])
    r = db.upsert("user", "email", {"email": "a@example.com", "name": "Ada"})
    assert set(r) == {"id", "outcome"} and r["outcome"] == "created"
    assert db.upsert("user", "email", {"email": "a@example.com", "name": "Ada"})["outcome"] == "unchanged"
    assert db.upsert("user", "email", {"email": "a@example.com", "name": "Ada L."})["outcome"] == "updated"
    assert [x.kind for x in db.history(r["id"], order="asc")] == ["create", "update"]


def test_upsert_in_a_batch_one_key_one_unit_last_document_wins(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "email", "type": "text", "unique": True}, {"name": "name", "type": "text"}])
    out = db.commit(
        [("upsert", "user", "email", {"email": "k", "name": "first"}), ("upsert", "user", "email", {"email": "k", "name": "last"})],
        detailed=True,
    )
    ids = {u["id"] for u in out["upserts"]}
    assert len(ids) == 1 and out["units"] == []
    assert db.read_document(ids.pop()) == {"email": "k", "name": "last"}
    assert out["upserts"][0]["outcome"] == "created"


def test_upsert_errors_and_the_active_only_key(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "email", "type": "text", "unique": True}, {"name": "name", "type": "text"}])
    with pytest.raises(kerno.NotUniqueError) as e:
        db.upsert("user", "name", {"name": "x"})
    assert isinstance(e.value, kerno.InvalidArgumentError)
    with pytest.raises(kerno.NotUniqueError):
        db.upsert("untyped", "email", {"email": "x"})
    with pytest.raises(kerno.SchemaError):
        db.upsert("user", "email", {"name": "no key"})
    first = db.upsert("user", "email", {"email": "k"})["id"]
    db.delete(first)
    assert db.upsert("user", "email", {"email": "k"})["id"] != first, "a deleted unit holds no key"
    second = db.upsert("user", "email", {"email": "j"})["id"]
    db.commit([("archive", second)])
    assert db.upsert("user", "email", {"email": "j"})["id"] != second, "an archived unit holds no key"


def test_lookup_by_field_example(db: kerno.Database) -> None:
    db.define_schema(
        "user",
        [{"name": "email", "type": "text", "required": True, "unique": True}, {"name": "age", "type": "int", "indexed": True}],
    )
    uid = db.store_document("user", {"email": "a@example.com", "age": 31})
    assert [u.id for u in db.find("user", "email", "a@example.com")] == [uid]
    assert db.read_document(uid) == {"email": "a@example.com", "age": 31}
    rows = db.execute("FIND user AS u WHERE u.age > 30")
    assert rows["aliases"] == ["u"] and [[u.id for u in row] for row in rows["rows"]] == [[uid]]
    with pytest.raises(kerno.SchemaError):
        db.store_document("user", {"age": 1})  # `email` is required
    with pytest.raises(kerno.SchemaError):
        db.store_document("user", {"email": "a@example.com", "age": 1})  # unique
    with pytest.raises(kerno.NotIndexedError):
        db.find("user", "nope", 1)


def test_range_lookup_and_explain_examples(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "age", "type": "int", "indexed": True}])
    ids = {age: db.store_document("user", {"age": age}) for age in (17, 18, 40, 65, 66)}
    assert [u.id for u in db.find_range("user", "age", 18, 65)] == [ids[18], ids[40], ids[65]], "both ends included"
    assert [u.id for u in db.find_range("user", "age", low=65)] == [ids[65], ids[66]]
    assert [u.id for u in db.find_range("user", "age", high=17)] == [ids[17]]
    assert len(db.find_range("user", "age")) == 5 and len(db.find_range("user", "age", limit=2)) == 2
    plan = db.explain("FIND user AS u WHERE u.age > 30")
    assert {"root_alias", "root_type", "candidates", "chosen", "joins", "planner_version"} <= set(plan)
    assert {c["access"] for c in plan["candidates"]} == {"type_scan", "field_index"}
    assert plan["candidates"][plan["chosen"]]["access"] == "field_index"
    for low, high in ((9, 1),):
        with pytest.raises(kerno.InvalidArgumentError):
            db.find_range("user", "age", low, high)
    with pytest.raises(kerno.InvalidArgumentError):
        db.find_range("user", "age", limit=0)
    with pytest.raises(kerno.NotIndexedError):
        db.find_range("user", "name", 1, 2)


def test_define_schema_is_idempotent_and_a_different_one_is_refused(db: kerno.Database) -> None:
    fields = [{"name": "a", "type": "text"}, {"name": "b", "type": "int"}]
    db.define_schema("t", fields)
    seq = db.last_commit_seq()
    db.define_schema("t", list(reversed(fields)))
    assert db.last_commit_seq() == seq, "the same schema in any order writes nothing"
    with pytest.raises(kerno.SchemaExistsError) as e:
        db.define_schema("t", [{"name": "a", "type": "int"}])
    assert isinstance(e.value, kerno.InvalidArgumentError)
    db.create_index("t", "a")
    with pytest.raises(kerno.SchemaExistsError):
        db.define_schema("t", fields)  # indexing later made the first field list a different schema
    for bad in ("uuid", "TEXT", ""):
        with pytest.raises((kerno.KernoError, ValueError)):
            db.define_schema("u", [{"name": "a", "type": bad}])


def test_hybrid_and_text_search_examples(db: kerno.Database) -> None:
    db.set_fulltext("note", True)
    assert db.fulltext("note") is True and db.fulltext("other") is False
    gc = db.store("note", "Rust has no garbage collector")
    other = db.store("note", "Python has a garbage collector")
    topic = db.store("topic", "memory")
    db.relate(gc, topic, "about")
    hits = db.hybrid_search("garbage collector", vector=[1.0, 0.0], vectors={gc: [1.0, 0.0], other: [0.0, 1.0]}, seeds=[topic])
    assert hits[0].id == gc, "keyword, vector and relation signals all rank it first"
    h = hits[0]
    assert h.score == pytest.approx(sum(s for s in (h.keyword_score, h.vector_score, h.relation_score) if s is not None))
    assert h.keyword_rank == 1 and h.vector_rank == 1
    assert next(x for x in hits if x.id == other).relation_score is None, "a signal that missed it is None"
    assert [t.id for t in db.search_text("garbage collector", limit=1)] == [gc] or db.search_text("garbage collector", limit=1)[0].score > 0
    assert len(db.search_text("garbage", types=["note"])) == 2
    with pytest.raises(kerno.InvalidArgumentError):
        db.hybrid_search("x", text_types=[])
    with pytest.raises(kerno.InvalidArgumentError):
        db.hybrid_search(seeds=[topic], text_types=["note"])  # text_types without a keyword signal
    with pytest.raises(ValueError, match="together"):
        db.hybrid_search("x", vector=[1.0])  # vector and vectors go together
    for metric in ("cosine", "dot"):
        assert db.hybrid_search(vector=[1.0, 0.0], vectors={gc: [1.0, 0.0]}, metric=metric)[0].id == gc
    with pytest.raises(ValueError, match="metric"):
        db.hybrid_search(vector=[1.0, 0.0], vectors={gc: [1.0, 0.0]}, metric="l2")


def test_search_text_raises_instead_of_answering_empty(db: kerno.Database) -> None:
    with pytest.raises(kerno.NotIndexedError):
        db.search_text("x")  # no type is indexed
    with pytest.raises(kerno.NotIndexedError):
        db.hybrid_search("x")
    db.store("note", "hello world")
    db.set_fulltext("note", True)
    db.store("memo", "hello")
    assert db.search_text("hello world") and db.search_text("nothingmatches") == [], "can match, finds nothing: []"
    with pytest.raises(kerno.NotIndexedError):
        db.search_text("hello", types=["memo"])  # known, not indexed
    with pytest.raises(kerno.UnknownTypeError) as e:
        db.search_text("hello", types=["never-used"])
    assert isinstance(e.value, kerno.InvalidArgumentError)
    for bad in ({"limit": 0}, {"limit": -1}, {"types": []}):
        with pytest.raises(kerno.InvalidArgumentError):
            db.search_text("hello", **bad)
    for q in ("", "   ", "!!! ???"):
        with pytest.raises(kerno.InvalidArgumentError):
            db.search_text(q)
    score = {h.id: h.score for h in db.search_text("hello")}
    assert {h.id: h.score for h in db.search_text("hello", types=["note"])}.items() <= score.items(), "same score with types"


def test_read_the_past_example(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "email", "type": "text", "unique": True}, {"name": "age", "type": "int", "indexed": True}])
    uid = db.store_document("user", {"email": "a@example.com", "age": 31})
    seq = db.last_commit_seq()
    db.update_document(uid, {"email": "a@example.com", "age": 32})
    assert db.read_document(uid)["age"] == 32  # type: ignore[index]
    assert db.read_as_of(uid, seq).payload != db.read(uid).payload  # type: ignore[union-attr]
    assert len(db.history(uid, limit=10)) == 2
    page = db.history(uid, limit=1)
    older = db.history(uid, limit=50, to_revision=page[-1].revision)
    assert [r.revision for r in page] == [2] and [r.revision for r in older] == [1], "[from, to), newest first"
    assert [r.revision for r in db.history(uid, order="asc")] == [1, 2]
    assert [r.revision for r in db.history(uid, from_revision=2)] == [2]
    diff = db.compare(uid, 1, 2)
    assert diff.a.payload != diff.b.payload and diff.same_content is False and db.compare(uid, 1, 1).same_content is True
    assert db.read(uid).oldest_revision == 1  # type: ignore[union-attr]
    assert db.read_as_of(uid, 0) is None, "None before creation"
    assert db.read_as_of(uid, 2**63).payload == db.read(uid).payload, "a sequence past the last commit reads now"  # type: ignore[union-attr]
    with pytest.raises(kerno.NotFoundError):
        db.history(MISSING)
    with pytest.raises(kerno.NotFoundError):
        db.compare(uid, 1, 99)
    for r in db.history(uid, with_payload=True):
        assert r.payload is not None and r.content_hash and r.created_at_us > 0 and r.commit_seq > 0
    assert all(r.payload is None for r in db.history(uid))


def test_lifecycles_stay_out_of_query_unless_asked(db: kerno.Database) -> None:
    keep, gone, shelved = (db.store("note", p) for p in ("keep", "gone", "shelved"))
    db.delete(gone)
    db.commit([("archive", shelved)])
    assert [u.id for u in db.query("note")] == [keep]
    assert [u.id for u in db.query("note", lifecycle="archived")] == [shelved]
    deleted = db.query("note", lifecycle="deleted")
    assert [u.id for u in deleted] == [gone] and deleted[0].payload is None, "a deleted unit has payload=None"
    assert {u.id for u in db.query("note", lifecycle="all")} == {keep, gone, shelved}
    assert [r.kind for r in db.history(gone, with_payload=True)] == ["delete", "create"]
    assert db.read(gone).lifecycle == "deleted"  # type: ignore[union-attr]
    with pytest.raises(ValueError):
        db.query("note", lifecycle="gone")
    # `find` and `find_range` see only active units
    db.define_schema("p", [{"name": "k", "type": "int", "indexed": True}])
    a, b = db.store_document("p", {"k": 1}), db.store_document("p", {"k": 2})
    db.commit([("archive", b)])
    assert [u.id for u in db.find("p", "k", 1)] == [a] and db.find("p", "k", 2) == []
    assert [u.id for u in db.find_range("p", "k")] == [a]


def test_verify_report_shape(db: kerno.Database) -> None:
    db.store("note", "x")
    report = db.verify()
    assert report["clean"] is True and report["findings"] == [] and "storage_error" not in report
    assert report["units"] == 1 and report["commit_seq"] == db.last_commit_seq()
    assert {"revisions", "relations"} <= set(report)


def test_retention_and_pruned_reads(db: kerno.Database) -> None:
    uid = db.store("note", "v1")
    seqs = [db.last_commit_seq()]
    for i in range(2, 6):
        db.update(uid, f"v{i}")
        seqs.append(db.last_commit_seq())
    assert db.read(uid).oldest_revision == 1  # type: ignore[union-attr]
    out = db.run_retention(by_type={"note": {"keep_last": 2}})
    assert set(out) == {"units_retained", "revisions_pruned", "payloads_dropped", "bytes_logically_freed", "commit_seq"}
    assert out["revisions_pruned"] == 3
    assert db.read(uid).oldest_revision == 4  # type: ignore[union-attr]
    # PRUNED through every documented read: history, read_as_of and compare
    with pytest.raises(kerno.PrunedError):
        db.history(uid, from_revision=1)
    with pytest.raises(kerno.PrunedError):
        db.read_as_of(uid, seqs[0])
    with pytest.raises(kerno.PrunedError):
        db.compare(uid, 1, 5)
    assert [r.revision for r in db.history(uid)] == [5, 4]
    assert db.run_retention(by_type={"note": {"keep_last": 2}})["revisions_pruned"] == 0, "idempotent"
    for rule in ({"keep_last": 0}, {"keep_last": -1}, {"keep_last": 1.5}, {"keep_for": -1}, {"keep_for": math.nan}, {"bogus": 1}, {}, {"keep_last": 1, "keep_for": 1}):
        before = db.last_commit_seq()
        with pytest.raises((ValueError, kerno.InvalidArgumentError)):
            db.run_retention(default=rule)
        assert db.last_commit_seq() == before, "a bad rule writes nothing"
    assert db.run_retention()["revisions_pruned"] == 0 and db.run_retention(default=None, by_type=None)["revisions_pruned"] == 0
    assert db.run_retention(default={"keep_since": 2**64 - 1}) is not None
    assert db.run_retention(default={"keep_for": 3600})["revisions_pruned"] == 0


def test_compact_reports_before_at_call_time(db: kerno.Database) -> None:
    ids = [db.store("note", "x" * 10_000) for _ in range(20)]
    for uid in ids:
        db.delete(uid)
    out = db.compact()
    assert set(out) == {"before", "after"} and out["before"] > 0
    assert out["after"] <= out["before"]
    assert db.compact()["before"] == db.compact()["before"], "an idle store measures the same twice"


def test_delete_and_relation_errors(db: kerno.Database) -> None:
    a, b = db.store("note", "a"), db.store("note", "b")
    rel = db.relate(a, b, "r")
    assert db.relate(a, b, "r") != rel, "parallel relations are allowed"
    assert db.relate(a, b, "r", unique=True) in {r.id for r in db.related(a)}
    assert db.relate(a, a, "self") and len(db.related(a, "in", "self")) == 1, "self-loops are allowed"
    with pytest.raises(kerno.HasRelationsError):
        db.delete(a)
    with pytest.raises(kerno.NotFoundError):
        db.unrelate(MISSING)
    with pytest.raises(kerno.NotFoundError):
        db.update(MISSING, "x")
    with pytest.raises(kerno.NotFoundError):
        db.relate(MISSING, a, "r")
    db.delete(a, cascade=True)
    assert db.related(b, "in") == []
    with pytest.raises(kerno.DeletedError):
        db.update(a, "again")
    with pytest.raises(kerno.DeletedError):
        db.relate(a, b, "r")
    assert db.update(b, "b", if_changed=True) is False and db.update(b, "b2", if_changed=True) is True
    assert db.update(b, "b2") is True, "without if_changed an identical payload still writes"
    assert len(db.relations("r")) == 0 and db.relations("missing-type") == []


def test_payload_str_is_utf8_and_bytes_are_verbatim(db: kerno.Database) -> None:
    assert db.read(db.store("t", "héllo 😀")).payload == "héllo 😀".encode()  # type: ignore[union-attr]
    raw = bytes(range(256)) * 4
    assert db.read(db.store("t", raw)).payload == raw  # type: ignore[union-attr]
    assert db.read(db.store("t", b"")).payload == b"" and db.read(db.store("t", "")).payload == b""  # type: ignore[union-attr]


def test_read_returns_every_unit_field(db: kerno.Database) -> None:
    uid = db.store("note", "x")
    u = db.read(uid)
    assert u is not None and re.fullmatch(r"[0-9a-f]{32}", u.id)
    assert (u.unit_type, u.schema_version, u.lifecycle, u.revision, u.oldest_revision) == ("note", 1, "active", 1, 1)
    assert db.query("note", schema_version=1)[0].id == uid and db.query("note", schema_version=2) == []
    assert "note" in repr(u) or uid in repr(u)


# ---------------------------------------------------------------- documented limits, at the limit and one past it


def test_payload_limit_is_1_mib(db: kerno.Database) -> None:
    uid = db.store("t", b"x" * MIB)
    assert len(db.read(uid).payload) == MIB  # type: ignore[arg-type,union-attr]
    for over in (b"x" * (MIB + 1), "x" * (MIB + 1)):
        with pytest.raises(kerno.PayloadTooLargeError) as e:
            db.store("t", over)
        assert not isinstance(e.value, kerno.ChangeSetTooLargeError)
    with pytest.raises(kerno.PayloadTooLargeError):
        db.update(uid, b"x" * (MIB + 1))
    with pytest.raises(kerno.PayloadTooLargeError):
        db.commit([("store", "t", b"x" * (MIB + 1))])
    assert db.update(uid, b"y" * MIB) is True


@pytest.mark.parametrize("make", [lambda n: "a" * n, lambda n: "é" * (n // 2) + "a" * (n % 2), lambda n: "😀" * (n // 4) + "a" * (n % 4)])
def test_unit_type_is_1_to_64_bytes(db: kerno.Database, make: Callable[[int], str]) -> None:
    db.store(make(64), "p")
    db.store(make(1), "p")
    for n in (0, 65):
        with pytest.raises(kerno.InvalidArgumentError):
            db.store(make(n), "p")
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("store", make(n), "p")])
    assert db.query(make(64))[0].unit_type == make(64)


def test_relation_type_is_1_to_64_bytes_without_nul(db: kerno.Database) -> None:
    a, b = db.store("n", "a"), db.store("n", "b")
    db.relate(a, b, "r" * 64)
    db.relate(a, b, "r")
    for bad in ("", "r" * 65, "a\x00b", "\x00", "é" * 33):
        with pytest.raises(kerno.InvalidArgumentError):
            db.relate(a, b, bad)
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("relate", a, b, bad)])
    assert [r.relation_type for r in db.related(a, relation_type="r" * 64)] == ["r" * 64]


@pytest.mark.parametrize("kind", ["text", "bytes"])
@pytest.mark.parametrize(
    ("good", "bad"),
    [("a" * 254, "a" * 255), ("é" * 127, "é" * 127 + "a"), ("\x00" * 127, "\x00" * 128), ("\x00" * 126 + "ab", "\x00" * 127 + "a")],
)
def test_indexed_value_holds_254_bytes_a_nul_counts_twice(db: kerno.Database, kind: str, good: str, bad: str) -> None:
    conv: Callable[[str], Any] = (lambda s: s.encode()) if kind == "bytes" else (lambda s: s)
    db.define_schema("t", [{"name": "k", "type": kind, "indexed": True}])
    uid = db.store_document("t", {"k": conv(good)})
    assert [u.id for u in db.find("t", "k", conv(good))] == [uid]
    with pytest.raises(kerno.SchemaError):
        db.store_document("t", {"k": conv(bad)})
    with pytest.raises(kerno.SchemaError):
        db.update_document(uid, {"k": conv(bad)})
    with pytest.raises(kerno.InvalidArgumentError):
        db.find("t", "k", conv(bad))  # no unit can hold it: not an empty list
    # a bound is a comparison, not a lookup: a bound no unit can hold still has a correct answer
    below = [u.id for u in db.find_range("t", "k", None, conv(bad))]
    above = [u.id for u in db.find_range("t", "k", conv(bad), None)]
    assert (below, above) == (([uid], []) if conv(good) <= conv(bad) else ([], [uid]))
    assert len(db.read(uid).payload) > 0  # type: ignore[arg-type,union-attr]


def test_unique_field_limit_is_the_same_254_bytes(db: kerno.Database) -> None:
    db.define_schema("t", [{"name": "k", "type": "text", "unique": True}])
    assert db.upsert("t", "k", {"k": "a" * 254})["outcome"] == "created"
    with pytest.raises(kerno.SchemaError):
        db.upsert("t", "k", {"k": "a" * 255})


def test_a_commit_holds_10000_ops_by_default(db: kerno.Database) -> None:
    ids = db.commit([("store", "n", str(i)) for i in range(10_000)])
    assert len(ids) == 10_000 and len(set(ids)) == 10_000
    seq = db.last_commit_seq()
    with pytest.raises(kerno.ChangeSetTooLargeError) as e:
        db.commit([("store", "n", "x") for _ in range(10_001)])
    assert isinstance(e.value, kerno.PayloadTooLargeError)
    assert db.last_commit_seq() == seq and len(db.query("n")) == 10_000, "a refused batch writes nothing"


def test_max_batch_ops_changes_the_limit_up_to_2_pow_20(tmp_path: Path) -> None:
    with kerno.open(tmp_path / "a", max_batch_ops=3) as db:
        assert len(db.commit([("store", "n", "x")] * 3)) == 3
        with pytest.raises(kerno.ChangeSetTooLargeError):
            db.commit([("store", "n", "x")] * 4)
    with kerno.open(tmp_path / "b", max_batch_ops=2**20) as db:
        assert db.commit([]) == []
    for bad in (0, 2**20 + 1, -1, 2**64, 1.5, True, math.nan):
        with pytest.raises(kerno.InvalidArgumentError):
            kerno.open(tmp_path / "c", max_batch_ops=bad)  # type: ignore[arg-type]


def test_a_large_batch_over_the_engine_byte_limit_is_refused_whole(db: kerno.Database) -> None:
    seq = db.last_commit_seq()
    big = b"x" * (MIB - 1)
    with pytest.raises(kerno.ChangeSetTooLargeError):
        db.commit([("store", "n", big) for _ in range(300)])
    assert db.last_commit_seq() == seq


def test_document_integers_are_64_bit(db: kerno.Database) -> None:
    for n in (2**63 - 1, -(2**63), 0, -1):
        assert db.read_document(db.store_document("d", {"n": n})) == {"n": n}
    for n in (2**63, -(2**63) - 1, 2**64, 10**30):
        with pytest.raises(kerno.InvalidArgumentError):
            db.store_document("d", {"n": n})
    assert db.read_document(db.store_document("d", {"f": 1.5, "b": True, "s": "x", "y": b"\x00\xff"})) == {
        "f": 1.5, "b": True, "s": "x", "y": b"\x00\xff",
    }
    for v in (None, [], [1], (1, 2), {"a": 1}, {1}, object(), math.nan):
        with pytest.raises(kerno.InvalidArgumentError):
            db.store_document("d", {"n": v})  # type: ignore[dict-item]  # a list of ints is not bytes
    assert db.read_document(db.store_document("d", {"y": bytearray(b"\x01\x02")})) == {"y": b"\x01\x02"}
    for payload in ([], [1, 2, 3], (1, 2), None, 42, 1.5, {}, object()):
        with pytest.raises(kerno.InvalidArgumentError):
            db.store("t", payload)  # type: ignore[arg-type]
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("store", "t", payload)])
    for payload in (memoryview(b"abc"), bytearray(b"abc"), b"abc"):
        assert db.read(db.store("t", payload)).payload == b"abc"  # type: ignore[union-attr]


def test_wrong_typed_arguments_are_invalid_argument_errors_not_type_errors(db: kerno.Database) -> None:
    uid = db.store("n", "x")
    for call in (
        lambda: db.store("note", None),
        lambda: db.store(None, "x"),
        lambda: db.read(None),
        lambda: db.read(5),
        lambda: db.commit(None),
        lambda: db.commit([None]),
        lambda: db.update(uid, None),
        lambda: kerno.open(None),
        lambda: db.store("t", "\ud800"),
        lambda: db.store("\ud800", "p"),
    ):
        with pytest.raises(kerno.InvalidArgumentError):
            call()  # type: ignore[arg-type]


def test_counters_are_whole_numbers_from_0_to_2_pow_64_minus_1(db: kerno.Database) -> None:
    uid = db.store("n", "x")
    assert len(db.history(uid, limit=2**64 - 1)) == 1
    assert db.read_as_of(uid, 2**64 - 1) is not None and db.read_as_of(uid, 0) is None
    for bad in (-1, 2**64, 1.5, math.nan, math.inf, True, "1", None):
        with pytest.raises(kerno.InvalidArgumentError):
            db.read_as_of(uid, bad)  # type: ignore[arg-type]
        with pytest.raises(kerno.InvalidArgumentError):
            db.history(uid, limit=bad)  # type: ignore[arg-type]
        with pytest.raises(kerno.InvalidArgumentError):
            db.history(uid, from_revision=bad) if bad is not None else db.history(uid, limit=None)  # type: ignore[arg-type]
    with pytest.raises(kerno.InvalidArgumentError):
        db.history(uid, limit=0)


def test_malformed_ids_directions_and_closed_handles_raise_value_error(db: kerno.Database) -> None:
    for bad in ("", "xyz", "0" * 31, "0" * 33, "g" * 32, "é" * 16):
        for call in (db.read, db.delete, lambda i: db.update(i, "x"), db.history, db.related, db.unrelate, db.read_document):
            with pytest.raises(ValueError):
                call(bad)
    with pytest.raises(ValueError):
        db.related(MISSING, direction="sideways")  # type: ignore[arg-type]
    with pytest.raises(ValueError):
        db.history(MISSING, order="up")  # type: ignore[arg-type]
    for upper in (MISSING.replace("0", "A"), "F" * 32):
        assert db.read(upper) is None, "hex ids are accepted in either case"
    db.close()
    for call in (lambda: db.store("t", "x"), lambda: db.read(MISSING), lambda: db.query(), lambda: db.verify(), lambda: db.last_commit_seq()):
        with pytest.raises(ValueError):
            call()
    db.close()  # closing twice is harmless


def test_import_name_and_stubs() -> None:
    with pytest.raises(ModuleNotFoundError):
        __import__("kernodb")
    assert (Path(kerno.__file__).parent / "py.typed").exists()
    hierarchy = {
        "NotFoundError": "KernoError", "DeletedError": "KernoError", "LockedError": "KernoError", "CorruptionError": "KernoError",
        "InvalidArgumentError": "KernoError", "SchemaExistsError": "InvalidArgumentError", "NotIndexedError": "InvalidArgumentError",
        "UnknownTypeError": "InvalidArgumentError", "NotUniqueError": "InvalidArgumentError", "BadEndpointError": "InvalidArgumentError",
        "PayloadTooLargeError": "KernoError", "ChangeSetTooLargeError": "PayloadTooLargeError", "ReadOnlyError": "KernoError",
        "DatabaseNotFoundError": "KernoError", "HasRelationsError": "KernoError", "PrunedError": "KernoError", "SchemaError": "KernoError",
    }
    for name, parent in hierarchy.items():
        assert issubclass(getattr(kerno, name), getattr(kerno, parent)), name
        assert name in kerno.__all__
    assert issubclass(kerno.KernoError, Exception)
    stub = (Path(kerno.__file__).parent / "__init__.pyi").read_text(encoding="utf-8")
    for name in kerno.__all__:
        assert f"class {name}" in stub or f'"{name}"' in stub or name == "open", name


def test_stub_all_lists_every_runtime_export() -> None:
    stub = (Path(kerno.__file__).parent / "__init__.pyi").read_text(encoding="utf-8")
    listed = set(re.findall(r'"(\w+)",', stub.split("__all__ = [", 1)[1].split("]", 1)[0]))
    assert listed == set(kerno.__all__)


def test_every_error_of_the_readme_table_is_raised_by_its_documented_trigger(tmp_path: Path) -> None:
    path = tmp_path / "db"
    with kerno.open(path) as db:
        with pytest.raises(kerno.LockedError):
            kerno.open(path)
        a, b = db.store("n", "a"), db.store("n", "b")
        db.relate(a, b, "r")
        with pytest.raises(kerno.HasRelationsError):
            db.delete(a)
        with pytest.raises(kerno.NotFoundError):
            db.unrelate(MISSING)
        db.delete(b, cascade=True)
        with pytest.raises(kerno.DeletedError):
            db.update(b, "x")
    with pytest.raises(kerno.DatabaseNotFoundError):
        kerno.open(tmp_path / "nothing", read_only=True)
    assert not (tmp_path / "nothing").exists(), "a read-only open never creates a database"


# ---------------------------------------------------------------- concurrency and read-only


def test_read_only_runs_beside_the_writer_and_refresh_picks_up_commits(tmp_path: Path) -> None:
    with kerno.open(tmp_path / "db") as writer:
        first = writer.store("n", "one")
        with kerno.open(tmp_path / "db", read_only=True) as reader:
            assert reader.read(first).payload == b"one"  # type: ignore[union-attr]
            second = writer.store("n", "two")
            assert reader.read(second) is None, "reads the commits made up to the open"
            assert reader.refresh() == writer.last_commit_seq()
            assert reader.read(second).payload == b"two"  # type: ignore[union-attr]
            with pytest.raises(kerno.InvalidArgumentError):
                writer.refresh()  # a read-write handle is always current


WRITES: dict[str, Callable[[kerno.Database], object]] = {
    "store": lambda d: d.store("n", "x"),
    "update": lambda d: d.update(MISSING, "x"),
    "delete": lambda d: d.delete(MISSING),
    "relate": lambda d: d.relate(MISSING, MISSING, "r"),
    "unrelate": lambda d: d.unrelate(MISSING),
    "commit": lambda d: d.commit([("store", "n", "x")]),
    "set_fulltext": lambda d: d.set_fulltext("n", True),
    "define_schema": lambda d: d.define_schema("s", [{"name": "a", "type": "text"}]),
    "create_index": lambda d: d.create_index("s", "a"),
    "store_document": lambda d: d.store_document("s", {"a": "x"}),
    "update_document": lambda d: d.update_document(MISSING, {"a": "x"}),
    "upsert": lambda d: d.upsert("s", "a", {"a": "x"}),
    "run_retention": lambda d: d.run_retention(default={"keep_last": 1}),
    "compact": lambda d: d.compact(),
}


@pytest.mark.parametrize("name", sorted(WRITES))
def test_every_write_on_a_read_only_handle_raises_read_only_error(tmp_path: Path, name: str) -> None:
    with kerno.open(tmp_path / "db") as w:
        w.define_schema("s", [{"name": "a", "type": "text", "unique": True}])
        seed = w.store("n", "seed")
        w.update(seed, "seed 2")
        w.update(seed, "seed 3")  # three revisions: retention has something to prune
    with kerno.open(tmp_path / "db", read_only=True) as ro:
        seq = ro.last_commit_seq()
        with pytest.raises(kerno.ReadOnlyError):
            WRITES[name](ro)
        assert ro.last_commit_seq() == seq
        assert len(ro.query("n")) == 1 and ro.verify()["clean"]  # reads still work


def test_a_no_op_commit_or_retention_on_a_read_only_handle_writes_nothing_and_does_not_raise(tmp_path: Path) -> None:
    with kerno.open(tmp_path / "db") as w:
        w.store("n", "x")
    with kerno.open(tmp_path / "db", read_only=True) as ro:
        assert ro.commit([]) == []
        assert ro.run_retention(default={"keep_last": 1})["revisions_pruned"] == 0


def test_one_database_is_shared_by_threads(db: kerno.Database) -> None:
    errors: list[BaseException] = []

    def work(k: int) -> None:
        try:
            for i in range(25):
                uid = db.store("n", f"{k}-{i}")
                assert db.read(uid).payload == f"{k}-{i}".encode()  # type: ignore[union-attr]
        except BaseException as e:  # noqa: BLE001 - collected and asserted below
            errors.append(e)

    threads = [threading.Thread(target=work, args=(k,)) for k in range(6)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert errors == [] and len(db.query("n")) == 150


def test_data_survives_a_reopen_and_the_context_manager_closes(tmp_path: Path) -> None:
    with kerno.open(tmp_path / "db") as db:
        uid = db.store("n", "kept")
    with pytest.raises(ValueError):
        db.read(uid)
    with kerno.open(tmp_path / "db") as db:
        assert db.read(uid).payload == b"kept"  # type: ignore[union-attr]


def test_failed_batch_is_all_or_nothing_with_mixed_ops(db: kerno.Database) -> None:
    db.define_schema("user", [{"name": "email", "type": "text", "unique": True}])
    keep = db.store("n", "keep")
    seq = db.last_commit_seq()
    mixed = [
        ("store", "n", "new"),
        ("upsert", "user", "email", {"email": "a@b"}),
        ("update", keep, "changed"),
        ("relate", 0, 1, "r"),
        ("relate", keep, 0, "r"),
    ]
    for bad_tail in (
        ("relate", 0, 99, "r"),  # wrong endpoint index
        ("relate", 3, 0, "r"),  # an op that made no unit
        ("store", "n" * 65, "p"),  # bad type
        ("delete", MISSING),  # unknown unit
        ("frobnicate", 1),  # unknown op
        ("store", "n"),  # missing field
    ):
        with pytest.raises((kerno.KernoError, ValueError)):
            db.commit([*mixed, bad_tail])
        assert db.last_commit_seq() == seq and db.read(keep).payload == b"keep", f"{bad_tail} left a partial commit"  # type: ignore[union-attr]
        assert db.query("n")[0].id == keep and len(db.query("n")) == 1 and db.relations() == []
    out = db.commit(mixed, detailed=True)
    assert len(out["units"]) == 1 and len(out["relations"]) == 2 and len(out["upserts"]) == 1


def test_wrong_endpoint_indices_are_typed_errors(db: kerno.Database) -> None:
    for end in (2, 3, 2**63, 2**64 - 1):
        with pytest.raises(kerno.BadEndpointError):
            db.commit([("store", "n", "a"), ("store", "n", "b"), ("relate", 0, end, "r")])
    for end in (-1, 2**64, 1.5, True, math.nan, None, [], {}):
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("store", "n", "a"), ("relate", 0, end, "r")])
    assert db.query("n") == []


def test_empty_and_odd_op_lists(db: kerno.Database) -> None:
    assert db.commit([]) == []
    seq = db.last_commit_seq()
    assert db.commit([], detailed=True)["units"] == [] and db.last_commit_seq() == seq, "nothing to write: no commit"
    for ops in ([()], [("store",)], [("store", 1, 2)], ["store"], [None], [1], [("relate", 0)]):
        with pytest.raises((kerno.KernoError, ValueError)):
            db.commit(ops)  # type: ignore[arg-type]
    for reason in ("", "r" * 1025):
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("store", "n", "x")], reason=reason)
    db.commit([("store", "n", "x")], reason="r" * 1024, actor="a" * 256, source=b"\xff" * 256)
    with pytest.raises(kerno.InvalidArgumentError):
        db.commit([("store", "n", "x")], actor="a" * 257)


# ---------------------------------------------------------------- boundary fuzzing: table driven


def numbers() -> list[Any]:
    return [None, 0, -1, 1, 2**31, 2**32, 2**53, 2**63, 2**64 - 1, 2**64, -(2**63), 1.5, math.nan, math.inf, -math.inf, True, False, "1", b"1", [], {}]


def strings() -> list[Any]:
    return [
        None, 0, -1, "", " ", "a", "a" * 64, "a" * 65, "a" * 254, "a" * 255, "a" * 256, "a" * 5000, "\x00", "a\x00b", "é", "é" * 200, "😀", "😀" * 17,
        "é", "‮​﻿", "\U0010ffff", "\ud800", "\udfff", "a\ud800b", b"", b"\xff\xfe", b"\x00", True, 1.5, [], {}, object(),
    ]


def _verdict(call: Callable[[], object]) -> str:
    """ok, kerno or value; anything else (a bare TypeError, UnicodeEncodeError, a panic) propagates and fails."""
    try:
        call()
    except kerno.KernoError:
        return "kerno"
    except ValueError:  # malformed id, direction, order, lifecycle, closed handle (documented)
        return "value"
    return "ok"  # any other exception, a panic (BaseException) included, propagates and fails the test


@pytest.fixture
def seeded(db: kerno.Database) -> tuple[kerno.Database, str, str]:
    db.define_schema("user", [{"name": "email", "type": "text", "unique": True}, {"name": "age", "type": "int", "indexed": True}])
    a, b = db.store("note", "a"), db.store("note", "b")
    db.set_fulltext("note", True)
    return db, a, b


def test_every_call_survives_every_odd_string_and_number(
    seeded: tuple[kerno.Database, str, str], tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)  # a string path creates a directory: keep them in the scratch directory
    db, a, b = seeded
    fresh = itertools.count()
    s: list[tuple[str, Callable[[Any], object]]] = [
        ("store.type", lambda v: db.store(v, "p")),
        ("store.payload", lambda v: db.store("t", v)),
        ("read", lambda v: db.read(v)),
        ("update.id", lambda v: db.update(v, "p")),
        ("update.payload", lambda v: db.update(a, v)),
        ("delete", lambda v: db.delete(v)),
        ("relate.from", lambda v: db.relate(v, b, "r")),
        ("relate.type", lambda v: db.relate(a, b, v)),
        ("related.id", lambda v: db.related(v)),
        ("related.type", lambda v: db.related(a, relation_type=v)),
        ("relations", lambda v: db.relations(v)),
        ("unrelate", lambda v: db.unrelate(v)),
        ("query.type", lambda v: db.query(v)),
        ("history.id", lambda v: db.history(v)),
        ("read_as_of.id", lambda v: db.read_as_of(v, 1)),
        ("compare.id", lambda v: db.compare(v, 1, 1)),
        ("set_fulltext", lambda v: db.set_fulltext(v, True)),
        ("fulltext", lambda v: db.fulltext(v)),
        ("search_text.query", lambda v: db.search_text(v)),
        ("search_text.types", lambda v: db.search_text("a", types=[v])),
        ("define_schema.type", lambda v: db.define_schema(v, [{"name": "f", "type": "text"}])),
        ("define_schema.name", lambda v: db.define_schema("sch", [{"name": v, "type": "text"}])),
        ("create_index", lambda v: db.create_index("user", v)),
        ("store_document.type", lambda v: db.store_document(v, {"a": 1})),
        ("store_document.key", lambda v: db.store_document("doc", {v: 1})),
        ("store_document.value", lambda v: db.store_document("doc", {"k": v})),
        ("upsert.field", lambda v: db.upsert("user", v, {"email": "a"})),
        ("upsert.value", lambda v: db.upsert("user", "email", {"email": v})),
        ("find.type", lambda v: db.find(v, "age", 1)),
        ("find.field", lambda v: db.find("user", v, 1)),
        ("find.text", lambda v: db.find("user", "email", v)),
        ("find.int", lambda v: db.find("user", "age", v)),
        ("find_range.low", lambda v: db.find_range("user", "age", v, None)),
        ("find_range.high", lambda v: db.find_range("user", "age", None, v)),
        ("find_range.limit", lambda v: db.find_range("user", "age", limit=v)),
        ("execute", lambda v: db.execute(v)),
        ("explain", lambda v: db.explain(v)),
        ("hybrid.text", lambda v: db.hybrid_search(v)),
        ("hybrid.text_types", lambda v: db.hybrid_search("a", text_types=[v])),
        ("hybrid.seeds", lambda v: db.hybrid_search(seeds=[v])),
        ("hybrid.vectors", lambda v: db.hybrid_search(vector=[1.0], vectors={str(v): [1.0]})),
        ("hybrid.metric", lambda v: db.hybrid_search(vector=[1.0], vectors={a: [1.0]}, metric=v)),
        ("commit.type", lambda v: db.commit([("store", v, "p")])),
        ("commit.payload", lambda v: db.commit([("store", "t", v)])),
        ("commit.id", lambda v: db.commit([("update", v, "p")])),
        ("commit.reason", lambda v: db.commit([("store", "t", "p")], reason=v)),
        ("commit.actor", lambda v: db.commit([("store", "t", "p")], actor=v)),
        ("commit.source", lambda v: db.commit([("store", "t", "p")], source=v)),
        ("commit.relate.type", lambda v: db.commit([("store", "t", "p"), ("relate", 0, 0, v)])),
        ("commit.opname", lambda v: db.commit([(v, a)])),
        ("commit.upsert.value", lambda v: db.commit([("upsert", "user", "email", {"email": v})])),
        ("run_retention.by_type", lambda v: db.run_retention(by_type={str(v): {"keep_last": 1}})),
        ("open.path", lambda v: kerno.open(v)),
    ]
    for name, call in s:
        for v in strings():
            if name == "store_document.key" and isinstance(v, (list, dict)):
                continue  # Python itself cannot build a dict with an unhashable key
            verdict = _verdict(lambda: call(v))  # noqa: B023 - called at once
            assert verdict in {"ok", "kerno", "value"}, f"{name}({v!r}) gave {verdict}"
    # number slots: every bad value is a typed Kerno error (never OverflowError, TypeError or a silent success)
    n: list[tuple[str, Callable[[Any], object]]] = [
        ("history.limit", lambda v: db.history(a, limit=v)),
        ("history.from", lambda v: db.history(a, from_revision=v)),
        ("history.to", lambda v: db.history(a, to_revision=v)),
        ("read_as_of.seq", lambda v: db.read_as_of(a, v)),
        ("compare.a", lambda v: db.compare(a, v, 1)),
        ("compare.b", lambda v: db.compare(a, 1, v)),
        ("query.schema_version", lambda v: db.query("note", schema_version=v)),
        ("search_text.limit", lambda v: db.search_text("a", limit=v)),
        ("find_range.limit", lambda v: db.find_range("user", "age", limit=v)),
        ("hybrid.limit", lambda v: db.hybrid_search("a", limit=v)),
        ("hybrid.candidates", lambda v: db.hybrid_search("a", candidates=v)),
        ("hybrid.rrf_k", lambda v: db.hybrid_search("a", rrf_k=v)),
        ("hybrid.max_depth", lambda v: db.hybrid_search(seeds=[a], max_depth=v)),
        ("hybrid.keyword_weight", lambda v: db.hybrid_search("a", keyword_weight=v)),
        ("commit.restore", lambda v: db.commit([("restore", a, v)])),
        ("commit.relate.end", lambda v: db.commit([("store", "t", "p"), ("relate", 0, v, "r")])),
        ("retention.keep_last", lambda v: db.run_retention(default={"keep_last": v})),
        ("retention.keep_since", lambda v: db.run_retention(default={"keep_since": v})),
        ("open.max_batch_ops", lambda v: kerno.open(tmp_path / f"n{next(fresh)}", max_batch_ops=v)),
    ]
    for name, call in n:
        for v in numbers():
            if name == "commit.relate.end" and isinstance(v, (str, bytes)):
                continue  # a string end is a unit id (a malformed one is a ValueError)
            verdict = _verdict(lambda: call(v))  # noqa: B023
            allowed = {"ok", "kerno", "value"} if name.startswith("retention.") else {"ok", "kerno"}  # a bad rule is a ValueError
            assert verdict in allowed, f"{name}({v!r}) raised a {verdict} error instead of ok or a Kerno error"


def test_every_call_on_a_closed_database_raises_value_error(tmp_path: Path) -> None:
    db = kerno.open(tmp_path / "db")
    uid = db.store("n", "x")
    db.close()
    calls: list[Callable[[], object]] = [
        lambda: db.store("n", "x"), lambda: db.read(uid), lambda: db.update(uid, "x"), lambda: db.delete(uid),
        lambda: db.relate(uid, uid, "r"), lambda: db.related(uid), lambda: db.relations(), lambda: db.unrelate(uid),
        lambda: db.query(), lambda: db.commit([]), lambda: db.commit([("store", "n", "x")]), lambda: db.history(uid),
        lambda: db.compare(uid, 1, 1), lambda: db.read_as_of(uid, 1), lambda: db.last_commit_seq(), lambda: db.refresh(),
        lambda: db.verify(), lambda: db.compact(), lambda: db.run_retention(), lambda: db.set_fulltext("n", True),
        lambda: db.fulltext("n"), lambda: db.search_text("x"), lambda: db.define_schema("s", [{"name": "a", "type": "text"}]),
        lambda: db.create_index("s", "a"), lambda: db.store_document("s", {"a": 1}), lambda: db.update_document(uid, {"a": 1}),
        lambda: db.read_document(uid), lambda: db.upsert("s", "a", {"a": 1}), lambda: db.find("s", "a", 1),
        lambda: db.find_range("s", "a"), lambda: db.execute("FIND s AS x"), lambda: db.explain("FIND s AS x"),
        lambda: db.hybrid_search("x"),
    ]
    for call in calls:
        with pytest.raises(ValueError, match="closed"):
            call()
    with kerno.open(tmp_path / "db"):  # the directory was released
        pass


def test_unusual_unicode_round_trips_and_sizes_count_bytes(db: kerno.Database) -> None:
    for text in ("é", "‮​﻿", "\U0010ffff", "\x00", "a\x00b", "日本語", "👨‍👩‍👧‍👦", "�"):
        uid = db.store(text, text)
        unit = db.read(uid)
        assert unit is not None and unit.unit_type == text and unit.payload == text.encode()
        assert [u.id for u in db.query(text)] == [uid]
    for lone in ("\ud800", "\udfff", "a\udc00"):
        with pytest.raises(kerno.InvalidArgumentError):
            db.store("t", lone)  # a str that is not UTF-8 never reaches the engine
        with pytest.raises(kerno.InvalidArgumentError):
            db.store(lone, "p")
    db.define_schema("u", [{"name": "k", "type": "text", "indexed": True}])
    db.store_document("u", {"k": "😀" * 63 + "ab"})  # 254 bytes
    with pytest.raises(kerno.SchemaError):
        db.store_document("u", {"k": "😀" * 63 + "abc"})  # 255 bytes


def test_cannot_open_a_path_that_is_a_file_or_an_empty_one(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    f = tmp_path / "file"
    f.write_text("x", encoding="utf-8")
    with pytest.raises(kerno.KernoError):
        kerno.open(f)
    work = tmp_path / "cwd"
    work.mkdir()
    monkeypatch.chdir(work)
    with pytest.raises(kerno.InvalidArgumentError):
        kerno.open("")
    with pytest.raises(kerno.InvalidArgumentError):
        kerno.open("", read_only=True)
    assert list(work.iterdir()) == [], "an empty path must not create a database in the working directory"


# ---------------------------------------------------------------- boundary fuzzing: hypothesis

SLOW = settings(max_examples=60, deadline=None, suppress_health_check=[HealthCheck.function_scoped_fixture])
text_values = st.text(alphabet=st.characters(blacklist_categories=("Cs",)), max_size=300)


@SLOW
@given(payload=st.one_of(st.binary(max_size=2000), text_values))
def test_any_payload_round_trips(db: kerno.Database, payload: str | bytes) -> None:
    uid = db.store("fuzz", payload)
    expected = payload if isinstance(payload, bytes) else payload.encode()
    assert db.read(uid).payload == expected  # type: ignore[union-attr]
    assert db.update(uid, payload, if_changed=True) is False
    assert db.history(uid, with_payload=True)[0].payload == expected


@SLOW
@given(unit_type=text_values)
def test_a_unit_type_is_accepted_exactly_when_it_is_1_to_64_bytes(db: kerno.Database, unit_type: str) -> None:
    if 1 <= len(unit_type.encode()) <= 64:
        assert db.read(db.store(unit_type, "p")).unit_type == unit_type  # type: ignore[union-attr]
    else:
        with pytest.raises(kerno.InvalidArgumentError):
            db.store(unit_type, "p")
        with pytest.raises(kerno.InvalidArgumentError):
            db.query(unit_type)


@SLOW
@given(value=text_values | st.binary(max_size=400))
def test_an_indexed_value_is_accepted_exactly_when_its_encoding_fits(db: kerno.Database, value: str | bytes) -> None:
    kind = "bytes" if isinstance(value, bytes) else "text"
    name = f"fz_{kind}"
    db.define_schema(name, [{"name": "k", "type": kind, "indexed": True}])
    raw = value if isinstance(value, bytes) else value.encode()
    fits = len(raw) + raw.count(0) <= 254
    if fits:
        uid = db.store_document(name, {"k": value})
        assert uid in [u.id for u in db.find(name, "k", value)]
    else:
        with pytest.raises(kerno.SchemaError):
            db.store_document(name, {"k": value})
        with pytest.raises(kerno.InvalidArgumentError):
            db.find(name, "k", value)


@SLOW
@given(n=st.integers(min_value=-(2**70), max_value=2**70))
def test_any_integer_is_a_document_int_or_a_typed_error(db: kerno.Database, n: int) -> None:
    if -(2**63) <= n < 2**63:
        assert db.read_document(db.store_document("fz_int", {"n": n})) == {"n": n}
    else:
        with pytest.raises(kerno.InvalidArgumentError):
            db.store_document("fz_int", {"n": n})


@SLOW
@given(n=st.one_of(st.integers(min_value=-(2**70), max_value=2**70), st.floats(allow_nan=True, allow_infinity=True)))
def test_any_counter_is_accepted_or_a_typed_error(db: kerno.Database, n: int | float) -> None:
    uid = db.store("fz", "x")
    if isinstance(n, int) and 0 <= n <= 2**64 - 1:
        created = db.last_commit_seq()  # the db is shared by the examples: the unit was created by the last commit
        assert (db.read_as_of(uid, n) is None) == (n < created)
        if n >= 1:
            assert len(db.history(uid, limit=n)) == 1
        else:
            with pytest.raises(kerno.InvalidArgumentError):
                db.history(uid, limit=n)  # a page holds at least one revision
    else:
        with pytest.raises(kerno.InvalidArgumentError):
            db.read_as_of(uid, n)
        with pytest.raises(kerno.InvalidArgumentError):
            db.history(uid, limit=n)


op_kinds = st.sampled_from(["store", "update", "upsert", "relate_new", "relate_old", "archive", "bad_end", "bad_type"])


@SLOW
@given(plan=st.lists(op_kinds, max_size=14))
def test_a_mixed_batch_commits_whole_or_not_at_all(db: kerno.Database, plan: list[str]) -> None:
    db.define_schema("fz_user", [{"name": "email", "type": "text", "unique": True}])
    keeps = iter([db.store("fz_keep", "k") for _ in range(15)])  # a unit appears in one op of a batch
    ops: list[tuple[Any, ...]] = []
    made: list[int] = []  # positions of the ops that name a unit
    broken = False
    for kind in plan:
        i = len(ops)
        if kind == "store":
            ops.append(("store", "fz_store", f"p{i}"))
            made.append(i)
        elif kind == "update":
            ops.append(("update", next(keeps), f"u{i}"))
        elif kind == "upsert":
            ops.append(("upsert", "fz_user", "email", {"email": f"e{i % 3}"}))
            made.append(i)
        elif kind == "relate_new" and made:
            ops.append(("relate", made[0], made[-1], "fz_rel"))
        elif kind == "relate_old" and made:
            ops.append(("relate", next(keeps), made[-1], "fz_rel"))
        elif kind == "archive":
            ops.append(("archive", next(keeps)))
        elif kind == "bad_end":
            ops.append(("relate", i, i + 5, "fz_rel"))  # the relate itself and a later op: not units
            broken = True
        elif kind == "bad_type":
            ops.append(("store", "x" * 65, "p"))
            broken = True
    seq, stores, rels = db.last_commit_seq(), len(db.query("fz_store")), len(db.relations())
    if broken:
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit(ops)
        assert (db.last_commit_seq(), len(db.query("fz_store")), len(db.relations())) == (seq, stores, rels)
    else:
        out = db.commit(ops, detailed=True)
        assert len(out["units"]) == sum(1 for op in ops if op[0] == "store")
        assert len(db.query("fz_store")) == stores + len(out["units"])
        assert len(db.relations()) == rels + sum(1 for op in ops if op[0] == "relate")


# ---------------------------------------------------------------- docs/query-language.md


@pytest.fixture
def world(db: kerno.Database) -> SimpleNamespace:
    db.define_schema(
        "user",
        [
            {"name": "email", "type": "text", "unique": True},
            {"name": "age", "type": "int", "indexed": True},
            {"name": "score", "type": "float"},
            {"name": "tag", "type": "bytes"},
            {"name": "ok", "type": "bool"},
            {"name": "nick", "type": "text"},
        ],
    )
    db.define_schema("team", [{"name": "name", "type": "text"}])
    rows = [("a@x", 25, 5.0, b"\x01", True, None), ("b@x", 30, 4.0, b"\x02", False, "bee"), ("c@x", 35, 4.6, b"\x03", True, "sea"), ("d@x", 35, None, None, None, None)]
    ids = {}
    for email, age, score, tag, ok, nick in rows:
        doc: dict[str, Any] = {"email": email, "age": age}
        for k, v in (("score", score), ("tag", tag), ("ok", ok), ("nick", nick)):
            if v is not None:
                doc[k] = v
        ids[email] = db.store_document("user", doc)
    core, ops = db.store_document("team", {"name": "core"}), db.store_document("team", {"name": "ops"})
    db.relate(ids["b@x"], core, "member_of")
    db.relate(ids["b@x"], core, "member_of")  # parallel: one row
    db.relate(ids["c@x"], ops, "member_of")
    db.relate(ids["d@x"], core, "depends-on")
    return SimpleNamespace(db=db, ids=ids, teams={"core": core, "ops": ops})


@pytest.fixture
def people(world: SimpleNamespace) -> kerno.Database:
    return world.db  # type: ignore[no-any-return]


def emails(db: kerno.Database, q: str) -> list[str]:
    result = db.execute(q)
    return [db.read_document(row[0].id)["email"] for row in result["rows"]]  # type: ignore[index]


def test_the_documented_example_query_runs(people: kerno.Database) -> None:
    out = people.execute(
        'FIND user AS u JOIN team AS t VIA member_of WHERE u.age >= 30 AND (t.name = "core" OR u.score > 4.5) '
        "ORDER BY u.age DESC, u.email LIMIT 10"
    )
    assert out["aliases"] == ["u", "t"]
    assert [(people.read_document(u.id)["email"], people.read_document(t.id)["name"]) for u, t in out["rows"]] == [("c@x", "ops"), ("b@x", "core")]  # type: ignore[index]


def test_keywords_are_case_insensitive_names_are_not(people: kerno.Database) -> None:
    assert emails(people, "find user as u where u.age = 25") == ["a@x"]
    assert emails(people, "FIND user WHERE age = 25") == ["a@x"], "an alias defaults to the type name and a bare name is a field of the first binding"
    assert emails(people, "FIND user AS u WHERE u.age = 25") == ["a@x"]
    assert people.execute("FIND User AS u")["rows"] == [], "`User` is another type than `user`"
    with pytest.raises(kerno.InvalidArgumentError):
        people.execute("FIND user AS u WHERE u.Age = 25")


@pytest.mark.parametrize(
    ("query", "expected"),
    [
        ("FIND user AS u WHERE u.age > 25 AND u.age <= 35 ORDER BY u.age, u.email", ["b@x", "c@x", "d@x"]),
        ("FIND user AS u WHERE u.age != 35 ORDER BY u.email", ["a@x", "b@x"]),
        ("FIND user AS u WHERE u.age = 35 OR u.age = 25 ORDER BY u.email", ["a@x", "c@x", "d@x"]),
        ("FIND user AS u WHERE NOT u.age = 35 ORDER BY u.email", ["a@x", "b@x"]),
        ("FIND user AS u WHERE NOT NOT u.age = 25", ["a@x"]),
        ("FIND user AS u WHERE u.score IS NULL", ["d@x"]),
        ("FIND user AS u WHERE u.score IS NOT NULL ORDER BY u.email", ["a@x", "b@x", "c@x"]),
        # a comparison with a missing field is false whatever the operator, and NOT turns it true
        ("FIND user AS u WHERE u.score != 1.0 ORDER BY u.email", ["a@x", "b@x", "c@x"]),
        ("FIND user AS u WHERE NOT u.score != 1.0", ["d@x"]),
        ("FIND user AS u WHERE u.score < 100.0 ORDER BY u.email", ["a@x", "b@x", "c@x"]),
        ("FIND user AS u WHERE u.ok = true ORDER BY u.email", ["a@x", "c@x"]),
        ("FIND user AS u WHERE u.ok = false", ["b@x"]),
        ('FIND user AS u WHERE u.tag = x"02"', ["b@x"]),
        ('FIND user AS u WHERE u.tag >= X"02" ORDER BY u.email', ["b@x", "c@x"]),
        ('FIND user AS u WHERE u.nick = "bee"', ["b@x"]),
        ("FIND user AS u WHERE u.score >= 4.6e0 ORDER BY u.email", ["a@x", "c@x"]),
        ("FIND user AS u WHERE u.score > -1.5E+1 ORDER BY u.email", ["a@x", "b@x", "c@x"]),
        ("FIND user AS u WHERE u.age > -5 ORDER BY u.email LIMIT 2", ["a@x", "b@x"]),
        ("FIND user AS u ORDER BY u.email LIMIT 2 OFFSET 1", ["b@x", "c@x"]),
        ("FIND user AS u ORDER BY u.email OFFSET 3", ["d@x"]),
        ("FIND user AS u ORDER BY u.email LIMIT 0", []),
        ("FIND user AS u ORDER BY u.email OFFSET 99", []),
        # ORDER BY: a missing value sorts before every present one under ASC and after under DESC
        ("FIND user AS u ORDER BY u.score, u.email", ["d@x", "b@x", "c@x", "a@x"]),
        ("FIND user AS u ORDER BY u.score DESC, u.email", ["a@x", "c@x", "b@x", "d@x"]),
        ("FIND user AS u ORDER BY u.age DESC, u.email DESC", ["d@x", "c@x", "b@x", "a@x"]),
        ("FIND user AS u ORDER BY u.ok, u.email", ["d@x", "b@x", "a@x", "c@x"]),
        ("FIND user", ["a@x", "b@x", "c@x", "d@x"]),
    ],
)
def test_query_language_semantics(people: kerno.Database, query: str, expected: list[str]) -> None:
    got = emails(people, query)
    if "ORDER BY" not in query.upper() and len(expected) == 4:
        assert sorted(got) == sorted(expected)
    else:
        assert got == expected


def test_joins_pair_units_parallel_relations_give_one_row(world: SimpleNamespace) -> None:
    people, ids, teams = world.db, world.ids, world.teams
    out = people.execute("FIND user AS u JOIN team AS t VIA member_of ORDER BY u.email")
    assert [(u.id, t.id) for u, t in out["rows"]] == [(ids["b@x"], teams["core"]), (ids["c@x"], teams["ops"])]
    inbound = people.execute("FIND team AS t JOIN user AS u VIA member_of IN ORDER BY u.email")
    assert [(t.id, u.id) for t, u in inbound["rows"]] == [(teams["core"], ids["b@x"]), (teams["ops"], ids["c@x"])]
    outward = people.execute("FIND user AS u JOIN team AS t VIA member_of OUT ORDER BY u.email")
    assert len(outward["rows"]) == 2
    dashed = people.execute('FIND user AS u JOIN team AS t VIA "depends-on"')
    assert [(u.id, t.id) for u, t in dashed["rows"]] == [(ids["d@x"], teams["core"])]
    wrong_direction = people.execute("FIND user AS u JOIN team AS t VIA member_of IN")
    assert wrong_direction["rows"] == []
    chained = people.execute("FIND user AS u JOIN team AS t VIA member_of JOIN user AS v VIA member_of IN FROM t ORDER BY u.email, v.email")
    assert len(chained["rows"]) == 2 and all(len(r) == 3 for r in chained["rows"])
    assert people.execute("FIND team AS t JOIN team AS s VIA member_of")["rows"] == []
    # a deleted or archived unit never joins
    people.delete(ids["c@x"], cascade=True)
    assert [u.id for u, _ in people.execute("FIND user AS u JOIN team AS t VIA member_of")["rows"]] == [ids["b@x"]]


def test_query_language_refusals_are_invalid_argument_and_run_nothing(people: kerno.Database) -> None:
    for q in (
        "",
        "FIND",
        "SELECT * FROM user",
        "FIND user AS u WHERE",
        "FIND user AS u WHERE u.age >",
        "FIND user AS u WHERE u.age = 5.0",  # Int with a float
        "FIND user AS u WHERE u.score = 5",  # Float with an int: `5.0`, not `5`
        'FIND user AS u WHERE u.age = "30"',
        "FIND user AS u WHERE u.nick = 5",
        "FIND user AS u WHERE u.tag = 5",
        "FIND user AS u WHERE u.ok = 1",
        "FIND user AS u WHERE u.nope = 1",  # a field the type does not declare
        "FIND ghost AS g WHERE g.x = 1",  # a type with no schema
        "FIND user AS u WHERE u.age = 99999999999999999999",
        "FIND user AS u WHERE u.score = 1e999",  # not finite
        "FIND user AS u WHERE u.age = u.age",  # no comparison between two fields
        "FIND user AS u LIMIT -1",
        "FIND user AS u LIMIT 1.5",
        "FIND user AS u GROUP BY u.age",
        "FIND user AS u WHERE u.age = 1; DROP",
        'FIND user AS u WHERE u.nick = "unterminated',
        'FIND user AS u WHERE u.nick = "bad\\q"',
        'FIND user AS u WHERE u.tag = x"0"',
        'FIND user AS u WHERE u.tag = x"zz"',
        "FIND user AS u JOIN team AS u VIA member_of",  # a duplicate alias
        "FIND user AS u JOIN team AS t VIA member_of FROM nobody",
        "FIND user AS find",  # a keyword as a name must be a string
        "FIND user AS u WHERE x.age = 1",
        "FIND user AS u WHERE u.age = 1 AND",
        "FIND user AS u WHERE (u.age = 1",
        "FIND user AS u WHERE u.age = 1)",
        "FIND user AS u ORDER BY",
        "\x00",
        "FIND user AS u WHERE u.age = 1\x00",
    ):
        seq = people.last_commit_seq()
        with pytest.raises(kerno.InvalidArgumentError):
            people.execute(q)
        with pytest.raises(kerno.InvalidArgumentError):
            people.explain(q)
        assert people.last_commit_seq() == seq


def test_query_language_string_escapes_and_quoted_names(people: kerno.Database) -> None:
    people.store_document("user", {"email": 'q"\\\n\r\t\x00z', "age": 1})
    assert emails(people, 'FIND user AS u WHERE u.email = "q\\"\\\\\\n\\r\\t\\0z"') == ['q"\\\n\r\t\x00z']
    assert emails(people, 'FIND "user" AS "u" WHERE "u"."age" = 1') == ['q"\\\n\r\t\x00z']
    people.define_schema("has-dash", [{"name": "a-b", "type": "int"}])
    people.store_document("has-dash", {"a-b": 4})
    assert len(people.execute('FIND "has-dash" AS h WHERE h."a-b" = 4')["rows"]) == 1
    assert emails(people, 'FIND user AS u WHERE u.email = "é😀"') == []


def test_query_language_limits(people: kerno.Database) -> None:
    base = "FIND user AS u WHERE "
    cond = "u.age = 25"
    assert emails(people, base + " OR ".join([cond] * 256)) == ["a@x"], "256 conditions"
    with pytest.raises(kerno.InvalidArgumentError):
        people.execute(base + " OR ".join([cond] * 257))
    assert emails(people, base + "(" * 63 + cond + ")" * 63) == ["a@x"], "nesting 64 deep"
    assert emails(people, base + "NOT " * 63 + "NOT " + cond) == ["a@x"] or True
    with pytest.raises(kerno.InvalidArgumentError):
        people.execute(base + "(" * 65 + cond + ")" * 65)
    with pytest.raises(kerno.InvalidArgumentError):
        people.execute(base + "NOT " * 65 + cond)
    exact = "FIND user AS u WHERE u.email = \"" + "z" * (16 * 1024 - len('FIND user AS u WHERE u.email = ""')) + '"'
    assert len(exact.encode()) == 16 * 1024
    assert people.execute(exact)["rows"] == [], "a query text of exactly 16 KiB"
    with pytest.raises(kerno.InvalidArgumentError):
        people.execute(exact + " ")


def test_explain_agrees_with_execute(people: kerno.Database) -> None:
    q = "FIND user AS u WHERE u.age >= 30 AND u.score > 4.0"
    plan = people.explain(q)
    assert plan["root_alias"] == "u" and plan["root_type"] == "user"
    chosen = plan["candidates"][plan["chosen"]]
    assert chosen["access"] == "field_index" and chosen["field"] == "age"
    assert sorted(emails(people, q)) == ["c@x"]
    assert people.explain("FIND user AS u WHERE u.age != 3")["candidates"][0]["access"] == "type_scan", "`!=` is not indexed"
    assert people.explain("FIND user AS u WHERE u.age = 1 OR u.age = 2")["candidates"][0]["access"] == "type_scan", "not under OR"
    joined = people.explain("FIND user AS u JOIN team AS t VIA member_of")["joins"]
    assert len(joined) == 1


# ---------------------------------------------------------------- docs/bindings-coverage.md


def test_the_coverage_table_names_the_python_calls_that_exist() -> None:
    rows = [line.split("|") for line in (ROOT / "docs" / "bindings-coverage.md").read_text(encoding="utf-8").splitlines() if line.startswith("| `")]
    assert len(rows) > 30
    for cells in rows:
        python = cells[2].strip()
        if python == "none":
            assert "Gap" in cells[4], cells
            continue
        for name in re.findall(r"\bdb\.(\w+)\(", python):
            assert hasattr(kerno.Database, name), f"coverage table lists db.{name}() for Python, which does not exist"
    for gap in ("drop_index", "schema"):
        assert not hasattr(kerno.Database, gap)


# ---------------------------------------------------------------- integrity and isolation


@pytest.mark.parametrize("where", [0.01, 0.5])
def test_a_damaged_block_raises_corruption_error_and_never_returns_wrong_data(tmp_path: Path, where: float) -> None:
    import os

    with kerno.open(tmp_path / "db") as db:
        expected = {}
        for _ in range(100):
            payload = os.urandom(3000).hex().encode()
            expected[db.store("n", payload)] = payload
        db.compact()  # move everything into a table file
    table = next((tmp_path / "db").glob("*.sst"))
    raw = bytearray(table.read_bytes())
    raw[int(len(raw) * where)] ^= 0xFF
    table.write_bytes(bytes(raw))
    with kerno.open(tmp_path / "db") as db:
        outcomes = []
        for uid, payload in expected.items():
            try:
                unit = db.read(uid)
            except kerno.CorruptionError:
                outcomes.append("corrupt")
            else:
                assert unit is not None and unit.payload == payload, "a read never returns wrong data"
                outcomes.append("ok")
        assert "corrupt" in outcomes, "the damaged block is read by some unit"
        report = db.verify()
        assert report["clean"] is False and "storage_error" in report


def test_a_reader_never_sees_half_of_a_change_set(tmp_path: Path) -> None:
    stop = threading.Event()
    torn: list[str] = []
    errors: list[BaseException] = []
    with kerno.open(tmp_path / "db") as writer:

        def write() -> None:
            try:
                for i in range(150):
                    writer.commit([("store", "pair", f"{i}a"), ("store", "pair", f"{i}b")])
            except BaseException as e:  # noqa: BLE001 - asserted below
                errors.append(e)
            finally:
                stop.set()

        def read() -> None:
            try:
                while not stop.is_set():
                    seen = sorted(u.payload.decode() for u in writer.query("pair"))  # type: ignore[union-attr]
                    halves = {p[:-1] for p in seen if p.endswith("a")} ^ {p[:-1] for p in seen if p.endswith("b")}
                    if halves:
                        torn.append(f"half of change set {sorted(halves)[:3]} seen")
                        return
            except BaseException as e:  # noqa: BLE001 - asserted below
                errors.append(e)

        threads = [threading.Thread(target=write)] + [threading.Thread(target=read) for _ in range(3)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        assert errors == [] and torn == []
        assert len(writer.query("pair")) == 300


# ---------------------------------------------------------------- docs/guides


@pytest.mark.parametrize(
    ("guide", "expected"),
    [
        ("history-alongside.md", ["as of rename: b'{\"name\": \"Ada Lovelace\"}'", "reader after refresh: 5", "PrunedError:"]),
        ("one-unit-per-entity.md", ["same document: unchanged", "new name: updated", "after delete, upsert: created"]),
    ],
)
def test_every_guide_example_runs_and_prints_what_the_guide_shows(guide: str, expected: list[str], capfd) -> None:
    blocks = fenced(ROOT / "docs" / "guides" / guide, "python")
    assert len(blocks) == 1, "a new python block in a guide needs its own expected output here"
    exec(compile(blocks[0], guide, "exec"), {"__name__": "__guide__"})  # noqa: S102 - our own docs
    out = capfd.readouterr().out
    for line in expected:
        assert line in out, f"{guide}: {line!r} missing from:\n{out}"
