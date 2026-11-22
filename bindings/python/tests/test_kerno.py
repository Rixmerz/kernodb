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


def test_refresh_shows_a_read_only_handle_the_commits_made_since_it_opened(tmp_path):
    path = tmp_path / "db"
    with kerno.open(path) as writer:
        first = writer.store("n", "one")
        with kerno.open(path, read_only=True) as reader:
            assert reader.read(first) is not None
            second = writer.store("n", "two")
            assert reader.read(second) is None  # not visible before refresh
            seq = reader.refresh()
            assert seq == writer.last_commit_seq() == reader.last_commit_seq()
            assert reader.read(second).payload == b"two"
        with pytest.raises(kerno.KernoError):
            writer.refresh()  # a read-write handle is always current


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
        with pytest.raises(kerno.ChangeSetTooLargeError):
            db.commit([("store", "n", b"\xff")] * 10_001)
    # One operation more than the default cap, so the raised cap is what lets it through.
    with kerno.open(tmp_path / "b", max_batch_ops=12_000) as db:
        out = db.commit([("store", "n", b"\xff")] * 6_000 + [("relate", 0, 1, "r")] * 4_001, detailed=True)
        assert len(out["units"]) == 6_000 and len(out["relations"]) == 4_001


def test_set_fulltext(tmp_path):
    # The keyword index is off by default and turned on per type (ADR-0023); a type that
    # already has units is reindexed (or cleared) in the same commit.
    with kerno.open(tmp_path / "db") as db:
        db.store("sym", '{"qname": "m.f"}')
        db.store("note", "hello")
        assert db.fulltext("note") is False
        db.set_fulltext("note", True)
        db.set_fulltext("note", True)  # same state: nothing written
        assert db.fulltext("note") is True
        assert db.verify()["clean"] is True
        db.set_fulltext("note", False)
        assert db.verify()["clean"] is True


def test_hybrid_search_fuses_keyword_vector_and_relation_signals(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.set_fulltext("doc", True)
        a = db.store("doc", "rust memory safety")
        b = db.store("doc", "python garbage collector")
        c = db.store("doc", "go goroutines")
        outside = db.store("plain", "rust but never keyword indexed")  # type without fulltext
        db.relate(b, c, "links")
        vectors = {a: [1.0, 0.0], b: [0.0, 1.0], c: [0.7, 0.7], outside: [1.0, 0.0]}

        keyword_only = db.hybrid_search("rust")
        assert [h.id for h in keyword_only] == [a]  # `outside` has the word but its type is not indexed
        assert keyword_only[0].keyword_rank == 1
        assert keyword_only[0].vector_score is None and keyword_only[0].relation_rank is None

        hits = db.hybrid_search("rust", vector=[1.0, 0.0], vectors=vectors, seeds=[b])
        by_id = {h.id: h for h in hits}
        assert set(by_id) == {a, c, outside}  # the seed itself is no relation hit; b is orthogonal
        top = hits[0]
        assert top.id == a  # the only unit two signals agree on
        assert top.keyword_rank == 1 and top.vector_rank in (1, 2) and top.relation_rank is None
        for h in hits:
            parts = [s for s in (h.keyword_score, h.vector_score, h.relation_score) if s is not None]
            assert h.score == pytest.approx(sum(parts))
        assert by_id[outside].keyword_score is None and by_id[outside].vector_rank is not None
        assert by_id[c].relation_rank == 1 and by_id[c].vector_rank == 3  # c is one hop from the seed b
        assert by_id[c].keyword_score is None
        assert [h.score for h in hits] == sorted((h.score for h in hits), reverse=True)

        # A weight of 0 switches its signal off; the weights scale the signal scores.
        no_vector = db.hybrid_search("rust", vector=[1.0, 0.0], vectors=vectors, vector_weight=0)
        assert [h.id for h in no_vector] == [a]
        weighted = db.hybrid_search("rust", keyword_weight=3)
        assert weighted[0].keyword_score == pytest.approx(3 * keyword_only[0].keyword_score)
        assert len(db.hybrid_search(vector=[1.0, 0.0], vectors=vectors, limit=2)) == 2


def test_hybrid_search_validates_its_input(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        uid = db.store("doc", "text")
        with pytest.raises(kerno.InvalidArgumentError):
            db.hybrid_search()  # no signal to run
        with pytest.raises(ValueError):
            db.hybrid_search(vector=[1.0])  # a query vector needs its vectors
        with pytest.raises(ValueError):
            db.hybrid_search("x", metric="euclid")
        with pytest.raises(kerno.InvalidArgumentError):
            db.hybrid_search(vector=[1.0, 0.0], vectors={uid: [1.0]})  # wrong dimension


def test_hybrid_search_keyword_leg_follows_set_fulltext(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.store("doc", "needle")
        with pytest.raises(kerno.NotIndexedError):  # off by default: a keyword search can never match
            db.hybrid_search("needle")
    with kerno.open(tmp_path / "db2") as db:
        db.set_fulltext("doc", True)
        uid = db.store("doc", "needle")
        assert [h.id for h in db.hybrid_search("needle")] == [uid]


def test_explain_shows_the_access_paths(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("user", [{"name": "age", "type": "int"}, {"name": "tag", "type": "text", "indexed": True}])
        for i in range(20):
            db.store_document("user", {"age": i, "tag": f"t{i}"})
        plan = db.explain("FIND user AS u WHERE u.age > 3")
        assert (plan["root_alias"], plan["root_type"]) == ("u", "user")
        assert [c["access"] for c in plan["candidates"]] == ["type_scan"]  # age is not indexed
        assert plan["candidates"][0]["field"] is None and plan["joins"] == []
        db.create_index("user", "age")
        plan = db.explain("FIND user AS u WHERE u.age = 3")
        accesses = [(c["access"], c["field"]) for c in plan["candidates"]]
        assert accesses == [("type_scan", None), ("field_index", "age")]
        chosen = plan["candidates"][plan["chosen"]]
        assert chosen["cost"] == min(c["cost"] for c in plan["candidates"])
        assert chosen["access"] == "field_index"  # one of 20 units: the index is cheaper
        assert db.execute("FIND user AS u WHERE u.age = 3")["rows"][0][0].payload  # explain ran nothing else
        with pytest.raises(kerno.InvalidArgumentError):
            db.explain("FIND")


def test_explain_lists_joins(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("a", [{"name": "n", "type": "int"}])
        db.define_schema("b", [{"name": "n", "type": "int"}])
        plan = db.explain("FIND a AS x JOIN b AS y VIA links")
        (join,) = plan["joins"]
        assert (join["alias"], join["unit_type"], join["relation_type"]) == ("y", "b", "links")
        assert join["direction"] == "out" and join["from"] == "x"


def test_find_range_is_inclusive_and_open_ended(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("user", [{"name": "age", "type": "int", "indexed": True}, {"name": "name", "type": "text"}])
        ids = {age: db.store_document("user", {"age": age, "name": str(age)}) for age in (5, 10, 20, 30, 40)}
        db.store_document("other", {"age": 20})  # another type is never returned

        def ages(**kw):
            return [db.read_document(u.id)["age"] for u in db.find_range("user", "age", **kw)]

        assert ages(low=10, high=30) == [10, 20, 30]  # both ends included
        assert ages(low=11, high=29) == [20]
        assert ages(low=20) == [20, 30, 40]  # open above
        assert ages(high=20) == [5, 10, 20]  # open below
        assert ages() == [5, 10, 20, 30, 40]
        assert ages(low=10, limit=2) == [10, 20]
        assert ages(low=31, high=39) == []
        db.delete(ids[20])
        assert ages(low=10, high=30) == [10, 30]  # deleted units drop out of the index
        with pytest.raises(kerno.InvalidArgumentError):
            db.find_range("user", "name", low="a")  # not indexed
        with pytest.raises(kerno.InvalidArgumentError):
            db.find_range("user", "age", low="x")  # wrong type


def test_find_range_orders_text_and_floats(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("m", [{"name": "w", "type": "float", "indexed": True}, {"name": "s", "type": "text", "indexed": True}])
        for w, s in ((-1.5, "b"), (0.5, "a"), (2.5, "c")):
            db.store_document("m", {"w": w, "s": s})
        docs = lambda units: [db.read_document(u.id) for u in units]  # noqa: E731
        assert [d["w"] for d in docs(db.find_range("m", "w", -2.0, 1.0))] == [-1.5, 0.5]
        assert [d["s"] for d in docs(db.find_range("m", "s", "b"))] == ["b", "c"]


def test_run_retention_prunes_old_revisions(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        note = db.store("note", "v1")
        first_seq = db.last_commit_seq()
        for i in range(2, 5):
            db.update(note, f"v{i}")
        log = db.store("log", "l1")
        db.update(log, "l2")
        before = db.last_commit_seq()

        # Nothing is pruned before retention runs; a refused rule writes nothing.
        assert db.read_as_of(note, first_seq).payload == b"v1"
        bad_rules = (
            {"keep_last": 0}, {}, {"keep_last": 1, "keep_for": 1.0}, {"keep_for": -1.0}, {"nope": 1},
            {"keep_last": -1}, {"keep_last": 1.5}, {"keep_last": "2"}, {"keep_last": 2**70}, {"keep_last": True},
            {"keep_last": None}, {"keep_since": -1}, {"keep_since": 1.5}, {"keep_for": "x"},
            {"keep_for": float("nan")}, {"keep_for": float("inf")}, {"keep_for": True}, {1: 1}, 3, "keep_last",
        )
        for bad in bad_rules:
            for call in (lambda: db.run_retention(by_type={"note": bad}), lambda: db.run_retention(default=bad)):
                with pytest.raises((kerno.InvalidArgumentError, ValueError)):
                    call()
        for by_type in (3, ["note"], {1: {"keep_last": 1}}):
            with pytest.raises(ValueError):
                db.run_retention(by_type=by_type)
        assert db.last_commit_seq() == before
        # `None` is the same as no rule: nothing to prune.
        assert db.run_retention(default=None, by_type={"note": None})["units_retained"] == 0
        assert db.last_commit_seq() == before

        # `note` keeps its newest two revisions; `log` has no rule and keeps everything.
        report = db.run_retention(by_type={"note": {"keep_last": 2}})
        assert (report["units_retained"], report["revisions_pruned"]) == (1, 2)
        assert report["commit_seq"] == db.last_commit_seq()
        assert db.read(note).oldest_revision == 3 and db.read(log).oldest_revision == 1
        assert [r.revision for r in db.history(note)] == [4, 3]
        with pytest.raises(kerno.PrunedError):
            db.read_as_of(note, first_seq)
        with pytest.raises(kerno.PrunedError):
            db.history(note, from_revision=1)
        with pytest.raises(kerno.PrunedError):
            db.compare(note, 1, 4)
        assert db.read(note).payload == b"v4"
        assert db.verify()["clean"] is True

        # Idempotent: a second run commits nothing.
        seq = db.last_commit_seq()
        again = db.run_retention(by_type={"note": {"keep_last": 2}})
        assert (again["units_retained"], again["commit_seq"]) == (0, None)
        assert db.last_commit_seq() == seq

        # A duration rule on every type: 0 seconds keeps only the revision in force now.
        db.run_retention(default={"keep_for": 0})
        assert db.read(note).oldest_revision == 4 and db.read(log).oldest_revision == 2
        assert db.verify()["clean"] is True

        # `keep_since` keeps what answers as-of reads from that commit on.
        other = db.store("memo", "m1")
        mid = db.last_commit_seq()
        db.update(other, "m2")
        db.update(other, "m3")
        db.run_retention(by_type={"memo": {"keep_since": mid}})
        assert db.read(other).oldest_revision == 1
        db.run_retention(by_type={"memo": {"keep_since": db.last_commit_seq()}})
        assert db.read(other).oldest_revision == 3


def test_history_bounds_and_order(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        uid = db.store("note", "v1")
        for i in range(2, 6):
            db.update(uid, f"v{i}")

        def revs(**kw):
            return [r.revision for r in db.history(uid, **kw)]

        assert revs() == [5, 4, 3, 2, 1]
        assert revs(order="asc") == [1, 2, 3, 4, 5]
        assert revs(from_revision=2, to_revision=4, order="asc") == [2, 3]  # [from, to)
        assert revs(from_revision=2, to_revision=4) == [3, 2]
        assert revs(from_revision=4) == [5, 4]
        assert revs(to_revision=3) == [2, 1]
        assert revs(to_revision=99) == [5, 4, 3, 2, 1]  # past the head: up to the head
        assert db.history(uid, from_revision=1, to_revision=2, with_payload=True)[0].payload == b"v1"
        # Paging: the last revision of a page is the next `to_revision`.
        page1 = revs(limit=2)
        page2 = revs(limit=2, to_revision=page1[-1])
        page3 = revs(limit=2, to_revision=page2[-1])
        assert (page1, page2, page3) == ([5, 4], [3, 2], [1])
        asc1 = revs(limit=2, order="asc")
        assert revs(limit=2, order="asc", from_revision=asc1[-1] + 1) == [3, 4]
        with pytest.raises(ValueError):
            db.history(uid, order="sideways")
        assert db.history("00" * 16, order="asc") == []


def test_compare_two_revisions(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        uid = db.store("note", "same")
        db.update(uid, "different")
        db.update(uid, "same")
        diff = db.compare(uid, 1, 2)
        assert (diff.a.revision, diff.b.revision) == (1, 2)
        assert (diff.a.payload, diff.b.payload) == (b"same", b"different")
        assert diff.same_content is False
        same = db.compare(uid, 1, 3)
        assert same.same_content is True and same.a.content_hash == same.b.content_hash
        assert db.compare(uid, 2, 1).a.revision == 2  # the order given is kept
        with pytest.raises(kerno.NotFoundError):
            db.compare(uid, 1, 9)
        with pytest.raises(kerno.NotFoundError):
            db.compare("00" * 16, 1, 2)


def test_oldest_revision_is_exposed(tmp_path):
    # Python cannot prune (retention has no binding, and `Config` has no retention setting),
    # so only the unpruned value is checked here.
    with kerno.open(tmp_path / "db") as db:
        uid = db.store("note", "v1")
        db.update(uid, "v2")
        unit = db.read(uid)
        assert (unit.oldest_revision, unit.revision) == (1, 2)
        assert db.history(uid, from_revision=unit.oldest_revision, order="asc")[0].revision == 1
        with pytest.raises(kerno.KernoError):
            db.history(uid, from_revision=0)
        db.delete(uid)
        assert db.read(uid).oldest_revision == 1


def test_query_lists_archived_and_deleted_units(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        live = db.store("note", "live")
        arch = db.store("note", "archived")
        gone = db.store("note", "gone")
        other_arch = db.store("topic", "archived topic")
        other_gone = db.store("topic", "gone topic")
        db.commit([("archive", arch), ("archive", other_arch), ("delete", gone), ("delete", other_gone)])

        def ids(**kw):
            return sorted(u.id for u in db.query(**kw))

        assert ids() == [live]
        assert ids(lifecycle="active") == [live]
        assert ids(lifecycle="archived") == sorted([arch, other_arch])
        assert ids(lifecycle="deleted") == sorted([gone, other_gone])
        assert ids(lifecycle="all") == sorted([live, arch, gone, other_arch, other_gone])
        assert ids(unit_type="note", lifecycle="archived") == [arch]
        assert ids(unit_type="topic", lifecycle="deleted") == [other_gone]
        assert ids(unit_type="note", lifecycle="all") == sorted([live, arch, gone])
        assert ids(unit_type="note", schema_version=2, lifecycle="all") == []
        (deleted,) = db.query("note", lifecycle="deleted")
        assert (deleted.lifecycle, deleted.payload) == ("deleted", None)
        (archived,) = db.query("note", lifecycle="archived")
        assert (archived.lifecycle, archived.payload) == ("archived", b"archived")
        assert [u.id for u in db.query(lifecycle="all")] == sorted(ids(lifecycle="all"))  # id order
        db.commit([("unarchive", arch)])
        assert ids(lifecycle="archived") == [other_arch]
        with pytest.raises(ValueError):
            db.query(lifecycle="gone")


def test_query_lifecycle_pages_past_one_engine_page(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        ids = db.commit([("store", "n", f"{i}") for i in range(300)])
        db.commit([("delete", i) for i in ids[::2]])
        assert sorted(u.id for u in db.query(lifecycle="deleted")) == sorted(ids[::2])
        assert len(db.query(lifecycle="all")) == 300
        assert len(db.query()) == 150


def _users(db):
    db.define_schema(
        "user",
        [
            {"name": "email", "type": "text", "unique": True},
            {"name": "name", "type": "text"},
            {"name": "age", "type": "int", "indexed": True},
        ],
    )


def test_upsert_creates_updates_and_leaves_an_unchanged_unit_alone(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        _users(db)
        made = db.upsert("user", "email", {"email": "a@x.io", "name": "Ada"})
        assert made["outcome"] == "created"
        same = db.upsert("user", "email", {"email": "a@x.io", "name": "Ada"})
        assert same == {"id": made["id"], "outcome": "unchanged"}
        assert [r.kind for r in db.history(made["id"], order="asc")] == ["create"]
        newer = db.upsert("user", "email", {"email": "a@x.io", "name": "Ada L."})
        assert newer == {"id": made["id"], "outcome": "updated"}
        assert [r.kind for r in db.history(made["id"], order="asc")] == ["create", "update"]
        assert db.read_document(made["id"])["name"] == "Ada L."
        other = db.upsert("user", "email", {"email": "b@x.io", "name": "Bob"})
        assert other["id"] != made["id"] and other["outcome"] == "created"
        assert len(db.find("user", "email", "a@x.io")) == 1


def test_upsert_refuses_a_field_that_is_not_unique_and_a_missing_key(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        _users(db)
        doc = {"email": "a@x.io", "name": "Ada", "age": 36}
        for field in ("age", "name", "nope"):  # indexed only, plain, not in the schema
            with pytest.raises(kerno.InvalidArgumentError):
                db.upsert("user", field, doc)
        with pytest.raises(kerno.InvalidArgumentError):
            db.upsert("bare", "email", doc)  # a type with no schema
        with pytest.raises(kerno.SchemaError):
            db.upsert("user", "email", {"name": "Ada"})
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("store", "note", "kept out"), ("upsert", "user", "name", doc)])
        assert db.query() == []
        with pytest.raises(ValueError):
            db.commit([("upsert", "user", "email", "not a dict")])
        with pytest.raises(ValueError):
            db.commit([("upsert", "user", "email")])


def test_upsert_in_a_commit_is_atomic_and_one_key_twice_is_one_unit(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        _users(db)
        note = db.store("note", "v1")
        out = db.commit(
            [
                ("upsert", "user", "email", {"email": "a@x.io", "name": "first"}),
                ("upsert", "user", "email", {"email": "b@x.io", "name": "Bob"}),
                ("upsert", "user", "email", {"email": "a@x.io", "name": "second"}),
                ("update", note, "v2"),
            ],
            detailed=True,
        )
        a, b, a_again = out["upserts"]
        assert a == a_again and a["id"] != b["id"] and a["outcome"] == "created"
        assert out["units"] == [] and out["commit_seq"] == db.last_commit_seq()
        assert db.read_document(a["id"])["name"] == "second"
        assert [r.kind for r in db.history(a["id"])] == ["create"]
        assert len(db.find("user", "email", "a@x.io")) == 1
        # An op after the upsert fails: the upsert is not visible either.
        seq = db.last_commit_seq()
        with pytest.raises(kerno.NotFoundError):
            db.commit([("upsert", "user", "email", {"email": "c@x.io"}), ("update", "0" * 32, "x")])
        assert db.find("user", "email", "c@x.io") == [] and db.last_commit_seq() == seq
        # Plain (not detailed) commit still returns the stored ids only.
        assert db.commit([("upsert", "user", "email", {"email": "a@x.io", "name": "third"})]) == []
        assert db.read_document(a["id"])["name"] == "third"


def test_upsert_keeps_one_unit_across_a_reopen(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        _users(db)
        first = db.upsert("user", "email", {"email": "a@x.io", "name": "Ada"})
        db.upsert("user", "email", {"email": "a@x.io", "name": "Ada L."})
    with kerno.open(tmp_path / "db") as db:
        again = db.upsert("user", "email", {"email": "a@x.io", "name": "Ada L."})
        assert again == {"id": first["id"], "outcome": "unchanged"}
        assert len(db.find("user", "email", "a@x.io")) == 1
        assert [r.kind for r in db.history(first["id"], order="asc")] == ["create", "update"]


GUIDES = Path(__file__).resolve().parents[3] / "docs" / "guides"


@pytest.mark.skipif(not GUIDES.is_dir(), reason="the docs are not next to the tests")
@pytest.mark.parametrize(
    ("guide", "expected"),
    [
        ("history-alongside.md", ["as of rename: b'{\"name\": \"Ada Lovelace\"}'", "reader after refresh: 5", "PrunedError:"]),
        ("one-unit-per-entity.md", ["same document: unchanged", "new name: updated", "after delete, upsert: created"]),
    ],
)
def test_the_guide_scripts_run_as_written(guide: str, expected: list[str], capfd: pytest.CaptureFixture[str]) -> None:
    text = (GUIDES / guide).read_text(encoding="utf-8")
    script = text.split("```python\n", 1)[1].split("```", 1)[0]
    exec(compile(script, guide, "exec"), {"__name__": "__guide__"})  # noqa: S102 - our own docs
    out = capfd.readouterr().out
    for line in expected:
        assert line in out, f"{guide}: {line!r} missing from the output:\n{out}"


# ---- gaps an outside user found in 1.0.0a3 (K1 to K10) ----


def _files(db):
    db.define_schema("file", [{"name": "path", "type": "text", "required": True, "unique": True}, {"name": "language", "type": "text"}])
    db.define_schema("sym", [{"name": "qn", "type": "text", "required": True, "unique": True}])


def test_a_relate_end_is_the_position_of_an_earlier_store_or_upsert(tmp_path):
    # K1: the op list is the index space, so an upsert in the middle is index 1, not skipped.
    with kerno.open(tmp_path / "db") as db:
        _files(db)
        out = db.commit(
            [
                ("store", "free", "A"),
                ("upsert", "file", "path", {"path": "g.py"}),
                ("store", "free", "B"),
                ("relate", 0, 1, "meant_for_g"),
            ],
            detailed=True,
        )
        a, _b = out["units"]
        (edge,) = db.related(a, "out", "meant_for_g")
        assert edge.to_id == out["upserts"][0]["id"]
        # K2: two typed upserts and a relation between them go in one commit.
        out = db.commit(
            [("upsert", "file", "path", {"path": "a.py"}), ("upsert", "sym", "qn", {"qn": "a.f"}), ("relate", 0, 1, "defines")],
            detailed=True,
        )
        file_id, sym_id = (u["id"] for u in out["upserts"])
        assert [r.to_id for r in db.related(file_id, "out", "defines")] == [sym_id]
        # An upsert that finds its unit unchanged is a valid end as well.
        again = db.commit([("upsert", "file", "path", {"path": "a.py"}), ("store", "free", "C"), ("relate", 1, 0, "of")], detailed=True)
        assert again["upserts"][0] == {"id": file_id, "outcome": "unchanged"}
        assert db.related(again["units"][0], "out", "of")[0].to_id == file_id


def test_a_wrong_relate_index_is_an_error_never_a_silent_link(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        _files(db)
        unit = db.store("free", "kept")
        seq = db.last_commit_seq()
        store = ("store", "free", "x")
        for ops in (
            [store, ("relate", 0, 2, "r"), store],  # a later op
            [store, ("relate", 0, 1, "r")],  # the relation itself
            [("update", unit, "y"), store, ("relate", 0, 1, "r")],  # an update creates no unit
            [store, ("relate", 0, 0, "r"), ("relate", 1, 0, "r")],  # a relation creates no unit
            [store, ("delete", unit), ("relate", 0, 1, "r")],  # nor does a delete
            [store, ("relate", 0, 99, "r")],  # past the end
        ):
            with pytest.raises(kerno.BadEndpointError) as caught:
                db.commit(ops)
            assert isinstance(caught.value, kerno.InvalidArgumentError)
            assert "Endpoint" not in str(caught.value), "no Rust enum names in the message"
            assert db.last_commit_seq() == seq, "a refused batch writes nothing"


@pytest.mark.parametrize("end", [-1, True, 2**64, 1.5, None])
def test_a_relate_end_that_is_not_an_index_or_an_id_is_invalid_argument(tmp_path, end):
    with kerno.open(tmp_path / "db") as db:
        with pytest.raises(kerno.InvalidArgumentError):
            db.commit([("store", "free", "x"), ("relate", 0, end, "r")])


def test_compact_before_is_the_size_at_call_time(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("row", [{"name": "body", "type": "text"}])
        for i in range(100):
            db.store_document("row", {"body": ("%d " % i) * 200})
        first = db.compact()
        assert first["before"] > 5_000, first  # was 0: the units were still in the write buffer
        for i in range(1000):
            db.store_document("row", {"body": ("%d " % i) * 200})
        second = db.compact()
        assert second["before"] > 5 * first["after"], (first, second)  # was equal to first["after"]


def test_the_indexed_text_limit_is_254_bytes_and_the_error_names_it(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("lim", [{"name": "key", "type": "text", "required": True, "unique": True}])
        db.upsert("lim", "key", {"key": "a" * 254})
        db.upsert("lim", "key", {"key": "é" * 127})  # characters do not count: 254 bytes
        for too_long in ("a" * 255, "é" * 128):
            with pytest.raises(kerno.SchemaError, match="254 bytes"):
                db.upsert("lim", "key", {"key": too_long})


def test_distinct_cases_raise_distinct_error_classes(tmp_path):
    hierarchy = {
        kerno.SchemaExistsError: kerno.InvalidArgumentError,
        kerno.NotIndexedError: kerno.InvalidArgumentError,
        kerno.NotUniqueError: kerno.InvalidArgumentError,
        kerno.BadEndpointError: kerno.InvalidArgumentError,
        kerno.ChangeSetTooLargeError: kerno.PayloadTooLargeError,
        kerno.ReadOnlyError: kerno.KernoError,
        kerno.DatabaseNotFoundError: kerno.KernoError,
    }
    for cls, parent in hierarchy.items():
        assert issubclass(cls, parent) and cls is not parent
    with kerno.open(tmp_path / "db", max_batch_ops=5) as db:
        db.define_schema("tag", [{"name": "label", "type": "text"}])
        db.define_schema("person", [{"name": "email", "type": "text", "unique": True}])
        with pytest.raises(kerno.SchemaExistsError):
            db.define_schema("tag", [{"name": "label", "type": "text"}, {"name": "extra", "type": "int"}])
        with pytest.raises(kerno.NotUniqueError):
            db.upsert("tag", "label", {"label": "a"})
        with pytest.raises(kerno.NotIndexedError):
            db.find("tag", "label", "a")
        with pytest.raises(kerno.NotIndexedError):
            db.find_range("tag", "label", "a", "z")
        with pytest.raises(kerno.ChangeSetTooLargeError):
            db.commit([("store", "n", "1")] * 6)
        with pytest.raises(kerno.InvalidArgumentError) as caught:
            db.store_document("tag", {"label": "x"}) and db.define_schema("ev", [{"name": "f", "type": "float"}]) or db.store_document("ev", {"f": float("nan")})
        assert type(caught.value) is kerno.InvalidArgumentError  # NaN stays the generic class
        with pytest.raises(kerno.InvalidArgumentError) as refresh:
            db.refresh()  # a handle that is not read-only
        assert type(refresh.value) is kerno.InvalidArgumentError
    path = tmp_path / "ro"
    with kerno.open(path) as w:
        w.store("n", "a")
        with kerno.open(path, read_only=True) as r:
            with pytest.raises(kerno.ReadOnlyError):
                r.store("n", "b")
            with pytest.raises(kerno.ReadOnlyError):
                r.commit([("store", "n", "b")])
    with pytest.raises(kerno.DatabaseNotFoundError) as missing:
        kerno.open(tmp_path / "missing" / "nothing", read_only=True)
    assert "create_if_missing" not in str(missing.value)


def test_define_schema_with_the_same_fields_is_idempotent(tmp_path):
    fields = [{"name": "email", "type": "text", "required": True, "unique": True}, {"name": "name", "type": "text"}]
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("person", fields)
        seq = db.last_commit_seq()
        db.define_schema("person", fields)
        db.define_schema("person", list(reversed(fields)))
        assert db.last_commit_seq() == seq, "the same schema writes nothing"
        with pytest.raises(kerno.SchemaExistsError):
            db.define_schema("person", fields + [{"name": "age", "type": "int"}])
        with pytest.raises(kerno.SchemaExistsError):
            db.define_schema("person", [{"name": "email", "type": "text"}, {"name": "name", "type": "text"}])


def test_searches_that_can_never_match_raise(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("lim", [{"name": "key", "type": "text", "required": True, "unique": True}])
        db.upsert("lim", "key", {"key": "a" * 254})
        assert len(db.find("lim", "key", "a" * 254)) == 1
        with pytest.raises(kerno.InvalidArgumentError, match="too long"):
            db.find("lim", "key", "b" * 300)  # no unit can hold it
        assert len(db.find_range("lim", "key", None, "b" * 300)) == 1  # a bound this long still means something
        with pytest.raises(kerno.NotIndexedError, match="set_fulltext"):
            db.hybrid_search(text="ada")  # no type is keyword-indexed
        db.set_fulltext("lim", True)
        assert db.hybrid_search(text="ada") == []  # indexed, nothing matches: a real empty answer


@pytest.mark.parametrize("value", [None, 2**64, 2**63, -(2**63) - 1, [1.5], {"a": 1}, object()])
def test_a_field_value_a_document_cannot_hold_raises_a_kerno_error(tmp_path, value):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("ev", [{"name": "i", "type": "int"}, {"name": "k", "type": "text", "unique": True}])
        for call in (
            lambda: db.store_document("ev", {"i": value}),
            lambda: db.update_document(db.store_document("ev", {"i": 1}), {"i": value}),
            lambda: db.upsert("ev", "k", {"k": "x", "i": value}),
            lambda: db.commit([("upsert", "ev", "k", {"k": "x", "i": value})]),
            lambda: db.find("ev", "i", value),
            lambda: db.find_range("ev", "i", value, None),
        ):
            with pytest.raises(kerno.KernoError):
                call()


def test_the_edges_of_a_64_bit_integer_are_accepted(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("ev", [{"name": "i", "type": "int", "indexed": True}])
        low, high = db.store_document("ev", {"i": -(2**63)}), db.store_document("ev", {"i": 2**63 - 1})
        assert [u.id for u in db.find("ev", "i", 2**63 - 1)] == [high]
        assert [u.id for u in db.find("ev", "i", -(2**63))] == [low]


@pytest.mark.parametrize("bad", [-1, 2**64, 1.5, "5", True])
def test_a_whole_number_argument_out_of_range_or_of_the_wrong_kind_is_invalid_argument(tmp_path, bad):
    # Issue #11: every unsigned parameter refuses a bad value with a Kerno error that names it.
    with kerno.open(tmp_path / "db") as db:
        uid = db.store("n", "a")
        db.define_schema("n2", [{"name": "v", "type": "int", "indexed": True}])
        calls = {
            "limit": [
                lambda: db.history(uid, limit=bad),
                lambda: db.hybrid_search("x", limit=bad),
                lambda: db.find_range("n2", "v", limit=bad),
            ],
            "from_revision": [lambda: db.history(uid, from_revision=bad)],
            "to_revision": [lambda: db.history(uid, to_revision=bad)],
            "a": [lambda: db.compare(uid, bad, 1)],
            "b": [lambda: db.compare(uid, 1, bad)],
            "commit_seq": [lambda: db.read_as_of(uid, bad)],
            "schema_version": [lambda: db.query("n", schema_version=bad)],
            "candidates": [lambda: db.hybrid_search("x", candidates=bad)],
            "rrf_k": [lambda: db.hybrid_search("x", rrf_k=bad)],
            "max_depth": [lambda: db.hybrid_search("x", max_depth=bad)],
            "keyword_weight": [lambda: db.hybrid_search("x", keyword_weight=bad)],
            "vector_weight": [lambda: db.hybrid_search("x", vector_weight=bad)],
            "relation_weight": [lambda: db.hybrid_search("x", relation_weight=bad)],
            "max_batch_ops": [lambda: kerno.open(tmp_path / "other", max_batch_ops=bad)],
            "revision": [lambda: db.commit([("restore", uid, bad)])],
        }
        seq = db.last_commit_seq()
        for name, group in calls.items():
            for call in group:
                with pytest.raises(kerno.InvalidArgumentError, match=name):
                    call()
        assert db.last_commit_seq() == seq, "a refused call writes nothing"


def test_a_pruned_read_says_which_number_is_a_revision_and_which_a_commit(tmp_path):
    with kerno.open(tmp_path / "db") as db:
        db.define_schema("cfg", [{"name": "v", "type": "int"}])
        uid = db.store_document("cfg", {"v": 0})
        seqs = [db.last_commit_seq()]
        for i in range(1, 5):
            db.update_document(uid, {"v": i})
            seqs.append(db.last_commit_seq())
        db.run_retention(default={"keep_last": 2})
        with pytest.raises(kerno.PrunedError) as caught:
            db.read_as_of(uid, seqs[0])
        message = str(caught.value)
        assert f"commit {seqs[0]}" in message and "revision 4" in message, message


def test_the_open_docstring_says_a_reader_opens_beside_a_writer():
    doc = kerno.open.__doc__
    assert "still holds" not in doc and "one handle at a time" not in doc
    assert "takes no lock" in doc and "refresh" in doc
