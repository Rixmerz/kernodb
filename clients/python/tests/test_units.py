"""Units and relations, against the real server."""

from __future__ import annotations

import pytest
from conftest import start_server, stop_server
from kerno_client import ApiError, KernoClient, Unit


def test_health(kerno: KernoClient) -> None:
    health = kerno.health()
    assert (health.status, health.version) == ("ok", "1.0.0-alpha.1")


def test_a_text_unit_round_trips(kerno: KernoClient) -> None:
    created = kerno.create_unit("note", "Rust has no GC")
    assert isinstance(created, Unit)
    assert (created.type, created.lifecycle, created.revision) == ("note", "active", 1)
    assert created.payload == "Rust has no GC"
    assert len(created.id) == 32
    assert created.created_at_us > 0
    assert kerno.get_unit(created.id) == created


def test_bytes_round_trip_and_utf8_bytes_come_back_as_text(kerno: KernoClient) -> None:
    raw = bytes([0, 255, 16])
    assert kerno.get_unit(kerno.create_unit("blob", raw).id).payload == raw
    assert kerno.create_unit("blob", bytearray(b"\xff\xfe")).payload == b"\xff\xfe"
    assert kerno.create_unit("blob", b"hi").payload == "hi"


def test_a_unit_is_updated_into_a_new_revision(kerno: KernoClient) -> None:
    unit = kerno.create_unit("note", "first")
    updated = kerno.update_unit(unit.id, "second")
    assert (updated.revision, updated.payload) == (2, "second")
    assert kerno.get_unit(unit.id) == updated


def test_expected_revision_guards_every_change(kerno: KernoClient) -> None:
    unit = kerno.create_unit("note", "v1")
    with pytest.raises(ApiError) as stale:
        kerno.update_unit(unit.id, "x", expected_revision=7)
    assert (stale.value.status, stale.value.code) == (409, "revision_conflict")
    assert kerno.get_unit(unit.id).revision == 1, "a refused update changes nothing"

    assert kerno.update_unit(unit.id, "v2", expected_revision=1).revision == 2
    for op in (kerno.archive_unit, kerno.unarchive_unit):
        with pytest.raises(ApiError) as e:
            op(unit.id, expected_revision=1)
        assert e.value.code == "revision_conflict"
    with pytest.raises(ApiError) as e:
        kerno.delete_unit(unit.id, expected_revision=1)
    assert e.value.code == "revision_conflict"
    assert kerno.archive_unit(unit.id, expected_revision=2).revision == 3
    assert kerno.unarchive_unit(unit.id, expected_revision=3).revision == 4
    kerno.delete_unit(unit.id, expected_revision=4)


def test_archive_unarchive_and_delete(kerno: KernoClient) -> None:
    unit = kerno.create_unit("note", "keep me")
    assert kerno.archive_unit(unit.id).lifecycle == "archived"
    assert list(kerno.iter_units("note")) == []
    assert kerno.unarchive_unit(unit.id).lifecycle == "active"
    assert kerno.delete_unit(unit.id) is None
    tombstone = kerno.get_unit(unit.id)
    assert (tombstone.lifecycle, tombstone.payload) == ("deleted", None)
    with pytest.raises(ApiError) as e:
        kerno.update_unit(unit.id, "x")
    assert (e.value.status, e.value.code) == (409, "conflict")


def test_an_unknown_unit_is_not_found(kerno: KernoClient) -> None:
    with pytest.raises(ApiError) as e:
        kerno.get_unit("00" * 16)
    assert (e.value.status, e.value.code) == (404, "not_found")
    assert "not found" in e.value.message
    with pytest.raises(ApiError) as e:
        kerno.get_unit("nope")
    assert (e.value.status, e.value.code) == (400, "invalid_request")


def test_units_are_listed_by_type_and_paged(kerno: KernoClient) -> None:
    notes = sorted(kerno.create_unit("note", f"n{i}").id for i in range(5))
    kerno.create_unit("task", "t")
    assert [u.id for u in kerno.list_units(type="note").units] == notes
    assert len(kerno.list_units().units) == 6

    first = kerno.list_units(type="note", limit=2)
    assert ([u.id for u in first.units], first.next) == (notes[:2], notes[1])
    last = kerno.list_units(type="note", limit=2, after=notes[3])
    assert ([u.id for u in last.units], last.next) == (notes[4:], None)
    assert [u.id for u in kerno.iter_units("note", page_size=2)] == notes
    assert [u.id for u in kerno.iter_units("note")] == notes
    with pytest.raises(ApiError) as e:
        kerno.list_units(limit=0)
    assert e.value.code == "invalid_request"


def test_relations_are_listed_from_both_ends(kerno: KernoClient) -> None:
    a, b, c = (kerno.create_unit("note", x) for x in "abc")
    outgoing = kerno.relate(b.id, c.id, "about")
    incoming = kerno.relate(a.id, b.id, "about")
    assert (outgoing.from_id, outgoing.to_id, outgoing.type, outgoing.provenance) == (b.id, c.id, "about", "declared")
    assert outgoing.direction is None

    assert [(r.id, r.direction) for r in kerno.relations(b.id, direction="out")] == [(outgoing.id, "out")]
    assert [(r.id, r.direction) for r in kerno.relations(b.id, direction="in")] == [(incoming.id, "in")]
    both = [(r.id, r.direction) for r in kerno.relations(b.id)]
    assert both == [(outgoing.id, "out"), (incoming.id, "in")]
    assert kerno.relations(b.id, direction="both") == kerno.relations(b.id)
    assert kerno.relations(b.id, type="other") == []
    kerno.relate(b.id, c.id, "cites")
    assert len(kerno.relations(b.id, direction="out", type="about")) == 1


def test_a_unit_with_relations_cannot_be_deleted_until_they_are(kerno: KernoClient) -> None:
    a, b = kerno.create_unit("note", "a"), kerno.create_unit("note", "b")
    edge = kerno.relate(a.id, b.id, "about")
    for unit in (a, b):
        with pytest.raises(ApiError) as e:
            kerno.delete_unit(unit.id)
        assert (e.value.status, e.value.code) == (409, "conflict")
    assert kerno.delete_relation(edge.id) is None
    assert kerno.relations(a.id) == []
    kerno.delete_unit(a.id)
    kerno.delete_unit(b.id)
    with pytest.raises(ApiError) as e:
        kerno.delete_relation(edge.id)
    assert e.value.code == "not_found"


def test_units_survive_a_graceful_stop(server_binary: str, server, kerno: KernoClient) -> None:
    unit = kerno.create_unit("note", "durable")
    assert stop_server(server) == 0
    again = start_server(server_binary, server.db, server.log)
    try:
        assert KernoClient(again.url).get_unit(unit.id) == unit
    finally:
        assert stop_server(again) == 0


def test_set_fulltext_turns_keyword_search_on_for_a_type(kerno):
    # "memo" is not in the suite's --fulltext list: off by default (Kerno 1.0.0-alpha.2).
    unit = kerno.create_unit("memo", "orchid greenhouse")
    assert kerno.get_fulltext("memo") is False
    assert not [h for h in kerno.search(text="orchid") if h.unit.id == unit.id]
    assert kerno.set_fulltext("memo", True) is True
    assert [h.unit.id for h in kerno.search(text="orchid")] == [unit.id]
    assert kerno.set_fulltext("memo", False) is False
