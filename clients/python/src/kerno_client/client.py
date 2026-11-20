"""The HTTP client."""

from __future__ import annotations

import json
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, Iterator, List, Optional, Sequence

from .errors import ApiError, TransportError
from .models import (
    FieldDef,
    GraphLeg,
    Health,
    Hit,
    Page,
    Payload,
    Plan,
    QueryResult,
    Relation,
    Schema,
    Unit,
    VectorLeg,
    Weights,
    encode_payload,
)


def _quote(segment: str) -> str:
    """A path segment: a unit type may hold any character, including ``/``."""
    return urllib.parse.quote(segment, safe="")


def _query_string(params: Dict[str, Any]) -> str:
    """``?a=1&b=2`` for the parameters that are set, or nothing."""
    given = {k: v for k, v in params.items() if v is not None}
    return "?" + urllib.parse.urlencode(given) if given else ""


class KernoClient:
    """A client of one kerno-server.

    >>> kerno = KernoClient("http://127.0.0.1:8080")
    >>> note = kerno.create_unit("note", "Rust has no GC")
    >>> kerno.get_unit(note.id).payload
    'Rust has no GC'

    Every method that talks to the server raises :class:`~kerno_client.ApiError` when the server
    refuses the call and :class:`~kerno_client.TransportError` when it cannot be reached.
    """

    def __init__(self, base_url: str = "http://127.0.0.1:8080", timeout: float = 30.0) -> None:
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    # ----------------------------------------------------------------- plumbing

    def _request(self, method: str, path: str, body: Any = None) -> Any:
        """The decoded JSON answer of one call; ``None`` for an empty answer (204)."""
        data = None if body is None else json.dumps(body).encode("utf-8")
        request = urllib.request.Request(f"{self.base_url}/v1{path}", data=data, method=method)
        request.add_header("Accept", "application/json")
        if data is not None:
            request.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                raw = response.read()
        except urllib.error.HTTPError as e:
            raise _api_error(e.code, e.read()) from None
        except (urllib.error.URLError, OSError) as e:
            raise TransportError(f"cannot reach {self.base_url}: {e}") from e
        return json.loads(raw) if raw else None

    # -------------------------------------------------------------------- health

    def health(self) -> Health:
        data = self._request("GET", "/health")
        return Health(data["status"], data["version"])

    # -------------------------------------------------------------------- units

    def create_unit(self, type: str, payload: Payload) -> Unit:
        """Stores a unit. ``payload`` is a ``str`` (text), ``bytes``, or a ``dict`` of fields for a
        unit type that has a schema."""
        body = {"type": type, "payload": encode_payload(payload)}
        return Unit.from_json(self._request("POST", "/units", body))

    def get_unit(self, id: str) -> Unit:
        """The unit; a deleted one is returned with ``lifecycle == "deleted"``."""
        return Unit.from_json(self._request("GET", f"/units/{_quote(id)}"))

    def list_units(self, type: Optional[str] = None, limit: Optional[int] = None, after: Optional[str] = None) -> Page:
        """One page of active units in id order; ``limit`` is 1 to 1000 (the server's default is 100)."""
        data = self._request("GET", "/units" + _query_string({"type": type, "limit": limit, "after": after}))
        return Page([Unit.from_json(u) for u in data["units"]], data["next"])

    def iter_units(self, type: Optional[str] = None, page_size: Optional[int] = None) -> Iterator[Unit]:
        """Every active unit, following the pages."""
        after: Optional[str] = None
        while True:
            page = self.list_units(type=type, limit=page_size, after=after)
            yield from page.units
            if page.next is None:
                return
            after = page.next

    def update_unit(self, id: str, payload: Payload, expected_revision: Optional[int] = None) -> Unit:
        """A new revision with ``payload``. With ``expected_revision`` the update is made only if the
        unit is at that revision, else :class:`~kerno_client.ApiError` with code ``revision_conflict``."""
        query = _query_string({"expected_revision": expected_revision})
        data = self._request("PUT", f"/units/{_quote(id)}{query}", {"payload": encode_payload(payload)})
        return Unit.from_json(data)

    def archive_unit(self, id: str, expected_revision: Optional[int] = None) -> Unit:
        query = _query_string({"expected_revision": expected_revision})
        return Unit.from_json(self._request("POST", f"/units/{_quote(id)}/archive{query}"))

    def unarchive_unit(self, id: str, expected_revision: Optional[int] = None) -> Unit:
        query = _query_string({"expected_revision": expected_revision})
        return Unit.from_json(self._request("POST", f"/units/{_quote(id)}/unarchive{query}"))

    def delete_unit(self, id: str, expected_revision: Optional[int] = None) -> None:
        """Deletes the unit logically. A unit that still has relations is refused
        (code ``conflict``): delete them first."""
        self._request("DELETE", f"/units/{_quote(id)}" + _query_string({"expected_revision": expected_revision}))

    # ---------------------------------------------------------------- relations

    def relate(self, from_id: str, to_id: str, type: str) -> Relation:
        body = {"from": from_id, "to": to_id, "type": type}
        return Relation.from_json(self._request("POST", "/relations", body))

    def relations(self, unit_id: str, direction: Optional[str] = None, type: Optional[str] = None) -> List[Relation]:
        """The relations of a unit; ``direction`` is ``"out"``, ``"in"`` or ``"both"`` (the default)."""
        query = _query_string({"direction": direction, "type": type})
        data = self._request("GET", f"/units/{_quote(unit_id)}/relations{query}")
        return [Relation.from_json(r) for r in data["relations"]]

    def delete_relation(self, id: str) -> None:
        self._request("DELETE", f"/relations/{_quote(id)}")

    # ------------------------------------------------------------------ schemas

    def define_schema(self, type: str, fields: Sequence[FieldDef]) -> Schema:
        """Gives a unit type its schema. A type has one for good."""
        body = {"type": type, "fields": [f.to_json() for f in fields]}
        return Schema.from_json(self._request("POST", "/schemas", body))

    def get_schema(self, type: str) -> Schema:
        return Schema.from_json(self._request("GET", f"/schemas/{_quote(type)}"))

    # -------------------------------------------------------------------- query

    def query(self, query: str) -> QueryResult:
        """Runs a query of the Kerno query language (``docs/query-language.md``)."""
        return QueryResult.from_json(self._request("POST", "/query", {"query": query}))

    def explain(self, query: str) -> Plan:
        """How ``query`` would run: its access paths with their costs, the one chosen, its joins."""
        return Plan.from_json(self._request("POST", "/query/explain", {"query": query}))

    # ------------------------------------------------------------------- search

    def search(
        self,
        text: Optional[str] = None,
        vector: Optional[VectorLeg] = None,
        graph: Optional[GraphLeg] = None,
        weights: Optional[Weights] = None,
        rrf_k: Optional[int] = None,
        candidates: Optional[int] = None,
        limit: Optional[int] = None,
    ) -> List[Hit]:
        """Hybrid retrieval: keywords (``text``), vectors and relations, fused by reciprocal rank.

        At least one leg must be given. Anything left ``None`` takes the server's default.
        """
        body: Dict[str, Any] = {
            "text": text,
            "vector": None if vector is None else vector.to_json(),
            "graph": None if graph is None else graph.to_json(),
            "weights": None if weights is None else weights.to_json(),
            "rrf_k": rrf_k,
            "candidates": candidates,
            "limit": limit,
        }
        data = self._request("POST", "/search", {k: v for k, v in body.items() if v is not None})
        return [Hit.from_json(h) for h in data["hits"]]


def _api_error(status: int, raw: bytes) -> ApiError:
    """The error of a refused call. The body is the server's error object, unless something else
    (a proxy) answered."""
    try:
        error = json.loads(raw)["error"]
        return ApiError(status, error["code"], error["message"])
    except (ValueError, KeyError, TypeError):
        return ApiError(status, None, raw.decode("utf-8", errors="replace")[:200] or "no body")
