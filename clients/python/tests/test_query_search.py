"""Schemas, the query language and hybrid search, against the real server."""

from __future__ import annotations

from typing import List

import pytest
from kerno_client import ApiError, FieldDef, GraphLeg, KernoClient, Unit, VectorLeg, Weights

USER_FIELDS = [
    FieldDef("email", "text", required=True, unique=True),
    FieldDef("age", "int", indexed=True),
    FieldDef("score", "float"),
    FieldDef("avatar", "bytes"),
    FieldDef("admin", "bool"),
]


def earns(weight: int, rrf_k: int, rank: int) -> int:
    """What one rank earns in a leg (the arithmetic of ``kerno_units::hybrid``)."""
    return weight * 1_000_000_000 // (rrf_k + rank)


def test_a_schema_is_defined_and_read_back_with_its_flags(kerno: KernoClient) -> None:
    schema = kerno.define_schema("user", USER_FIELDS)
    by_name = {f.name: f for f in schema.fields}
    assert schema.type == "user"
    assert by_name["email"] == FieldDef("email", "text", required=True, unique=True, indexed=True)
    assert by_name["age"] == FieldDef("age", "int", indexed=True)
    assert by_name["avatar"] == FieldDef("avatar", "bytes")
    assert by_name["score"] == FieldDef("score", "float")
    assert by_name["admin"] == FieldDef("admin", "bool")
    assert sorted(by_name) == ["admin", "age", "avatar", "email", "score"]
    assert kerno.get_schema("user") == schema


def test_schema_errors_carry_their_codes(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    with pytest.raises(ApiError) as again:
        kerno.define_schema("user", [FieldDef("x", "int")])
    assert (again.value.status, again.value.code) == (400, "invalid_request")
    with pytest.raises(ApiError) as missing:
        kerno.get_schema("nothing")
    assert (missing.value.status, missing.value.code) == (404, "not_found")
    # A unit type is any text: it is escaped in the path.
    kerno.define_schema("my type/1", [FieldDef("x", "int")])
    assert kerno.get_schema("my type/1").type == "my type/1"


def test_a_typed_unit_holds_fields_of_every_type(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    fields = {"email": "ada@x", "age": 36, "score": 4.5, "avatar": b"\x00\xff", "admin": True}
    created = kerno.create_unit("user", fields)
    assert created.payload == fields
    assert kerno.get_unit(created.id).payload == fields
    assert isinstance(created.payload, dict) and isinstance(created.payload["avatar"], bytes)
    assert kerno.list_units(type="user").units[0].payload == fields


def test_an_integer_for_a_float_field_is_a_float(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    unit = kerno.create_unit("user", {"email": "a@x", "score": 5})
    assert isinstance(unit.payload, dict)
    assert unit.payload["score"] == 5.0 and isinstance(unit.payload["score"], float)


def test_the_schema_refuses_what_does_not_fit(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    kerno.create_unit("user", {"email": "ada@x"})
    for bad in ({"age": 3}, {"email": "b@x", "age": "old"}, {"email": "b@x", "zzz": 1}, {"email": "ada@x"}):
        with pytest.raises(ApiError) as e:
            kerno.create_unit("user", bad)
        assert (e.value.status, e.value.code) == (422, "schema_violation"), bad
    assert len(list(kerno.iter_units("user"))) == 1
    with pytest.raises(ApiError) as e:
        kerno.create_unit("free", {"a": 1})
    assert (e.value.status, e.value.code) == (400, "invalid_request")


def _user(kerno: KernoClient, email: str, age: int) -> Unit:
    return kerno.create_unit("user", {"email": email, "age": age})


def test_a_query_filters_orders_and_pages(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    for email, age in [("a@x", 20), ("b@x", 30), ("c@x", 40), ("d@x", 50)]:
        _user(kerno, email, age)

    def ages(query: str) -> List[int]:
        result = kerno.query(query)
        assert result.aliases == ["u"]
        return [row[0].payload["age"] for row in result.rows]  # type: ignore[index]

    assert ages("FIND user AS u ORDER BY u.age DESC") == [50, 40, 30, 20]
    assert ages("FIND user AS u WHERE u.age >= 30 AND u.age < 50 ORDER BY u.age") == [30, 40]
    assert ages("FIND user AS u ORDER BY u.age LIMIT 2 OFFSET 1") == [30, 40]
    assert ages('FIND user AS u WHERE u.email = "zz"') == []


def test_a_query_joins_through_relations(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    kerno.define_schema("team", [FieldDef("name", "text", unique=True)])
    core = kerno.create_unit("team", {"name": "core"})
    ada, bob = _user(kerno, "ada@x", 36), _user(kerno, "bob@x", 29)
    kerno.relate(ada.id, core.id, "member_of")

    result = kerno.query('FIND user AS u JOIN team AS t VIA member_of WHERE t.name = "core"')
    assert result.aliases == ["u", "t"]
    assert [[unit.id for unit in row] for row in result.rows] == [[ada.id, core.id]]
    assert result.rows[0][1].payload == {"name": "core"}
    assert bob.id not in {unit.id for row in result.rows for unit in row}
    assert len(kerno.query("FIND team AS t JOIN user AS u VIA member_of IN").rows) == 1


def test_a_query_reports_the_access_path_it_used(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    for i in range(8):
        _user(kerno, f"u{i}@x", 20 + 3 * i)
    indexed = kerno.query("FIND user AS u WHERE u.age = 29")
    assert (indexed.access.kind, indexed.access.field) == ("field_index", "age")
    scan = kerno.query("FIND user AS u WHERE u.score IS NULL")
    assert (scan.access.kind, scan.access.field) == ("type_scan", None)


def test_explain_lists_the_candidates_and_the_chosen_one(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    kerno.define_schema("team", [FieldDef("name", "text")])
    for i in range(8):
        _user(kerno, f"u{i}@x", 20 + 3 * i)
    plan = kerno.explain("FIND user AS u WHERE u.age = 29")
    assert (plan.root_alias, plan.root_type, plan.chosen, plan.joins) == ("u", "user", 1, [])
    assert [c.access.kind for c in plan.candidates] == ["type_scan", "field_index"]
    assert plan.candidates[1].access.field == "age"
    assert plan.candidates[1].cost < plan.candidates[0].cost

    joined = kerno.explain("FIND user AS u JOIN team AS t VIA member_of JOIN user AS v VIA member_of IN FROM t")
    assert [(j.alias, j.unit_type, j.relation_type, j.direction, j.from_alias) for j in joined.joins] == [
        ("t", "team", "member_of", "out", "u"),
        ("v", "user", "member_of", "in", "t"),
    ]


def test_a_bad_query_says_why(kerno: KernoClient) -> None:
    kerno.define_schema("user", USER_FIELDS)
    for method in (kerno.query, kerno.explain):
        with pytest.raises(ApiError) as e:
            method("FIND")
        assert (e.value.status, e.value.code) == (400, "invalid_request")
        assert "byte 4" in e.value.message
        with pytest.raises(ApiError) as e:
            method("FIND user WHERE nope = 1")
        assert "declares no such field" in e.value.message


def test_the_keyword_leg_ranks_by_text(kerno: KernoClient) -> None:
    rust = kerno.create_unit("note", "rust rust rust ownership")
    mixed = kerno.create_unit("note", "rust and python")
    kerno.create_unit("note", "python only")
    hits = kerno.search(text="rust")
    assert [h.id for h in hits] == [rust.id, mixed.id]
    assert (hits[0].ranks.keyword, hits[0].ranks.vector, hits[0].ranks.graph) == (1, None, None)
    assert hits[0].score == earns(1, 60, 1)
    assert hits[1].score == earns(1, 60, 2)
    assert hits[0].unit == rust


def test_a_search_returns_ten_hits_unless_asked_for_more(kerno: KernoClient) -> None:
    for i in range(12):
        kerno.create_unit("note", f"common word {i}")
    assert len(kerno.search(text="common")) == 10
    assert len(kerno.search(text="common", limit=3)) == 3
    assert len(kerno.search(text="common", limit=50)) == 12


def test_the_graph_leg_follows_relations_from_its_seeds(kerno: KernoClient) -> None:
    a, b, c, d = (kerno.create_unit("note", x) for x in "abcd")
    kerno.relate(a.id, b.id, "next")
    kerno.relate(b.id, c.id, "next")
    kerno.relate(c.id, d.id, "next")
    assert [h.id for h in kerno.search(graph=GraphLeg([a.id]))] == [b.id, c.id]
    assert [h.id for h in kerno.search(graph=GraphLeg([a.id], max_depth=3))] == [b.id, c.id, d.id]
    # Both directions is the default: one hop from `b` reaches `a` and `c`, two reach `d` as well.
    assert {h.id for h in kerno.search(graph=GraphLeg([b.id], max_depth=1))} == {a.id, c.id}
    assert {h.id for h in kerno.search(graph=GraphLeg([b.id]))} == {a.id, c.id, d.id}
    assert [h.id for h in kerno.search(graph=GraphLeg([b.id], direction="out"))] == [c.id, d.id]
    assert [h.id for h in kerno.search(graph=GraphLeg([b.id], direction="in"))] == [a.id]
    assert kerno.search(graph=GraphLeg([b.id], relation_types=["other"])) == []
    assert kerno.search(graph=GraphLeg([a.id]))[0].ranks.graph == 1


def test_the_vector_leg_ranks_the_vectors_it_carries(kerno: KernoClient) -> None:
    y, z = kerno.create_unit("note", "y"), kerno.create_unit("note", "z")
    # `y` is long and 45 degrees off the query, `z` is short and exactly on it.
    vectors = {y.id: [5.0, 5.0], z.id: [0.9, 0.0]}
    by_cosine = kerno.search(vector=VectorLeg([1.0, 0.0], vectors))
    assert [h.id for h in by_cosine] == [z.id, y.id]
    assert by_cosine[0].ranks.vector == 1
    assert [h.id for h in kerno.search(vector=VectorLeg([1.0, 0.0], vectors, metric="cosine"))] == [z.id, y.id]
    assert [h.id for h in kerno.search(vector=VectorLeg([1.0, 0.0], vectors, metric="dot"))] == [y.id, z.id]


def test_the_legs_are_fused_and_weighted(kerno: KernoClient) -> None:
    both = kerno.create_unit("note", "rust memory")
    keyword_only = kerno.create_unit("note", "rust")
    seed = kerno.create_unit("note", "seed")
    kerno.relate(seed.id, both.id, "about")
    graph = GraphLeg([seed.id])

    fused = kerno.search(text="rust", graph=graph)
    assert fused[0].id == both.id
    ranks = fused[0].ranks
    assert fused[0].score == earns(1, 60, ranks.keyword or 0) + earns(1, 60, ranks.graph or 0)
    assert keyword_only.id in {h.id for h in fused} and seed.id not in {h.id for h in fused}

    graph_only = kerno.search(text="rust", graph=graph, weights=Weights(keyword=0))
    assert [h.id for h in graph_only] == [both.id]
    heavy = kerno.search(graph=graph, weights=Weights(graph=3), rrf_k=0, limit=1)
    assert heavy[0].score == earns(3, 0, 1)
    partial = kerno.search(text="rust", graph=graph, weights=Weights(graph=2))
    assert partial[0].score == earns(1, 60, partial[0].ranks.keyword or 0) + earns(2, 60, partial[0].ranks.graph or 0)


def test_candidates_bounds_what_each_leg_contributes(kerno: KernoClient) -> None:
    for i in range(101):
        kerno.create_unit("note", f"bulk {i}")
    assert len(kerno.search(text="bulk", limit=1000)) == 100
    assert len(kerno.search(text="bulk", limit=1000, candidates=7)) == 7


def test_a_search_that_cannot_run_is_refused(kerno: KernoClient) -> None:
    unit = kerno.create_unit("note", "a")
    cases = [
        dict(),
        dict(text="x", weights=Weights(keyword=0)),
        dict(text="x", limit=0),
        dict(graph=GraphLeg([])),
        dict(graph=GraphLeg(["nope"])),
        dict(vector=VectorLeg([1.0, 0.0], {unit.id: [1.0]})),
    ]
    for kwargs in cases:
        with pytest.raises(ApiError) as e:
            kerno.search(**kwargs)  # type: ignore[arg-type]
        assert (e.value.status, e.value.code) == (400, "invalid_request"), kwargs
