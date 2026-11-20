"""What the client sends and how it reads a server that is not the real one.

The tests that talk to the real server are in the other files. These use a stub that records the
requests, to check what a real server would hide: which parameters are left out, how a path is
escaped, what an answer that is not a kerno-server error becomes.
"""

from __future__ import annotations

import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, Iterator, List, Optional, Tuple

import pytest
from kerno_client import ApiError, GraphLeg, KernoClient, TransportError, VectorLeg, Weights
from kerno_client.models import encode_payload

UNIT = {
    "id": "00" * 16, "type": "note", "lifecycle": "active", "revision": 1,
    "created_at_us": 1, "updated_at_us": 1, "payload": {"text": "x"},
}


class Stub:
    """A server that answers every request the same way and remembers what it was asked."""

    def __init__(self) -> None:
        self.requests: List[Tuple[str, str, Optional[Any]]] = []
        self.status = 200
        self.body: bytes = b"{}"
        #: Answers to give first, one per request, before falling back to `status` and `body`.
        self.queue: List[Tuple[int, bytes]] = []
        self.delay = 0.0

        stub = self

        class Handler(BaseHTTPRequestHandler):
            def _answer(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(length) if length else b""
                # The raw request target: `self.path` has a leading `//` collapsed since Python 3.12.
                target = self.requestline.split(" ")[1]
                stub.requests.append((self.command, target, json.loads(raw) if raw else None))
                time.sleep(stub.delay)
                status, body = stub.queue.pop(0) if stub.queue else (stub.status, stub.body)
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            do_GET = do_POST = do_PUT = do_DELETE = _answer

            def log_message(self, *args: object) -> None:
                pass

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}"
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def reply(self, status: int, body: Any) -> None:
        self.status = status
        self.body = body if isinstance(body, bytes) else json.dumps(body).encode()

    @property
    def last(self) -> Tuple[str, str, Optional[Any]]:
        return self.requests[-1]


@pytest.fixture
def stub() -> Iterator[Stub]:
    s = Stub()
    yield s
    s.httpd.shutdown()
    s.httpd.server_close()


def test_a_parameter_left_out_is_not_sent(stub: Stub) -> None:
    stub.reply(200, {"units": [], "next": None})
    client = KernoClient(stub.url)
    client.list_units()
    assert stub.last[:2] == ("GET", "/v1/units")
    client.list_units(type="a b", limit=5, after="ff")
    assert stub.last[:2] == ("GET", "/v1/units?type=a+b&limit=5&after=ff")


def test_a_revision_of_zero_is_still_sent(stub: Stub) -> None:
    stub.reply(200, UNIT)
    client = KernoClient(stub.url)
    client.update_unit("ab", "x")
    assert stub.last[1] == "/v1/units/ab"
    client.update_unit("ab", "x", expected_revision=0)
    assert stub.last[1] == "/v1/units/ab?expected_revision=0"
    client.archive_unit("ab", expected_revision=0)
    assert stub.last[1] == "/v1/units/ab/archive?expected_revision=0"
    client.unarchive_unit("ab", expected_revision=3)
    assert stub.last[1] == "/v1/units/ab/unarchive?expected_revision=3"
    stub.reply(204, b"")
    assert client.delete_unit("ab", expected_revision=0) is None
    assert stub.last[:2] == ("DELETE", "/v1/units/ab?expected_revision=0")


def test_iter_units_follows_next_until_it_is_null(stub: Stub) -> None:
    pages = [{"units": [UNIT], "next": "aa"}, {"units": [UNIT, UNIT], "next": "bb"}, {"units": [], "next": None}]
    stub.queue = [(200, json.dumps(page).encode()) for page in pages]
    assert len(list(KernoClient(stub.url).iter_units("note", page_size=2))) == 3
    assert [r[1] for r in stub.requests] == [
        "/v1/units?type=note&limit=2",
        "/v1/units?type=note&limit=2&after=aa",
        "/v1/units?type=note&limit=2&after=bb",
    ]


def test_a_path_segment_is_escaped_whole(stub: Stub) -> None:
    stub.reply(200, {"type": "a/b c", "fields": []})
    KernoClient(stub.url).get_schema("a/b c")
    assert stub.last[1] == "/v1/schemas/a%2Fb%20c"
    stub.reply(200, {"relations": []})
    KernoClient(stub.url).relations("ab", direction="out", type="x y")
    assert stub.last[1] == "/v1/units/ab/relations?direction=out&type=x+y"


def test_a_relation_is_sent_with_the_server_names(stub: Stub) -> None:
    stub.reply(201, {"id": "r", "from": "a", "to": "b", "type": "about", "provenance": "declared"})
    relation = KernoClient(stub.url).relate("a", "b", "about")
    assert stub.last == ("POST", "/v1/relations", {"from": "a", "to": "b", "type": "about"})
    assert (relation.from_id, relation.to_id) == ("a", "b")


def test_a_search_sends_only_what_was_given(stub: Stub) -> None:
    stub.reply(200, {"hits": []})
    client = KernoClient(stub.url)
    assert client.search(text="rust") == []
    assert stub.last == ("POST", "/v1/search", {"text": "rust"})
    client.search(
        text="t",
        vector=VectorLeg([1.0], {"u": [1.0]}),
        graph=GraphLeg(["s"], direction="out", max_depth=3),
        weights=Weights(keyword=0, graph=2),
        rrf_k=0,
        candidates=5,
        limit=1,
    )
    assert stub.last[2] == {
        "text": "t",
        "vector": {"query": [1.0], "vectors": {"u": [1.0]}},
        "graph": {"seeds": ["s"], "direction": "out", "max_depth": 3},
        "weights": {"keyword": 0, "graph": 2},
        "rrf_k": 0,
        "candidates": 5,
        "limit": 1,
    }
    client.search(vector=VectorLeg([1.0], {}, metric="dot"))
    assert stub.last[2] == {"vector": {"query": [1.0], "vectors": {}, "metric": "dot"}}


def test_a_server_error_becomes_an_api_error(stub: Stub) -> None:
    stub.reply(404, {"error": {"code": "not_found", "message": "unit x not found"}})
    with pytest.raises(ApiError) as e:
        KernoClient(stub.url).get_unit("x")
    assert (e.value.status, e.value.code, e.value.message) == (404, "not_found", "unit x not found")
    assert str(e.value) == "404 not_found: unit x not found"


@pytest.mark.parametrize("body", [b"<html>Bad Gateway</html>", b"", b'{"error": 3}', b'{"detail": "x"}', b"[1]"])
def test_an_answer_that_is_not_a_server_error_still_raises_an_api_error(stub: Stub, body: bytes) -> None:
    stub.reply(502, body)
    with pytest.raises(ApiError) as e:
        KernoClient(stub.url).health()
    assert (e.value.status, e.value.code) == (502, None)
    assert e.value.message


def test_an_unreachable_server_is_a_transport_error() -> None:
    with pytest.raises(TransportError) as e:
        KernoClient("http://127.0.0.1:1", timeout=2).health()
    assert "127.0.0.1:1" in str(e.value)


def test_a_slow_server_times_out_as_a_transport_error(stub: Stub) -> None:
    stub.delay = 1.0
    with pytest.raises(TransportError):
        KernoClient(stub.url, timeout=0.2).health()


def test_the_base_url_may_end_with_a_slash(stub: Stub) -> None:
    stub.reply(200, {"status": "ok", "version": "1"})
    assert KernoClient(stub.url + "/").health().version == "1"
    assert stub.last[1] == "/v1/health"


def test_payloads_encode_as_the_server_expects() -> None:
    assert encode_payload("hi") == {"text": "hi"}
    assert encode_payload(b"\x00\xff") == {"hex": "00ff"}
    assert encode_payload(bytearray(b"\x01")) == {"hex": "01"}
    assert encode_payload({"a": 1, "b": b"\x02", "c": "x"}) == {"fields": {"a": 1, "b": {"hex": "02"}, "c": "x"}}
    with pytest.raises(TypeError):
        encode_payload(3)  # type: ignore[arg-type]


def test_a_unit_decodes_each_kind_of_payload() -> None:
    from kerno_client import Unit

    def unit(payload: Optional[Dict[str, Any]]) -> Unit:
        return Unit.from_json({**UNIT, "payload": payload})

    assert unit({"text": "x"}).payload == "x"
    assert unit({"hex": "00ff"}).payload == b"\x00\xff"
    assert unit({"fields": {"a": 1, "b": {"hex": "02"}, "c": {"hex": "0", "x": 1}}}).payload == {
        "a": 1,
        "b": b"\x02",
        "c": {"hex": "0", "x": 1},
    }
    assert unit(None).payload is None
