"""The values the client returns and takes, and their JSON forms.

A unit's payload is a Python value: ``str`` for text, ``bytes`` for raw bytes, a ``dict`` for the
typed record of a unit type that has a schema, ``None`` for a deleted unit. Bytes that happen to be
valid UTF-8 are read back as ``str``: the server cannot tell them apart.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Union

Fields = Dict[str, Any]
Payload = Union[str, bytes, Fields]


def _drop_none(values: Dict[str, Any]) -> Dict[str, Any]:
    return {k: v for k, v in values.items() if v is not None}


def encode_payload(payload: Payload) -> Dict[str, Any]:
    """The JSON of a payload to write."""
    if isinstance(payload, str):
        return {"text": payload}
    if isinstance(payload, (bytes, bytearray, memoryview)):
        return {"hex": bytes(payload).hex()}
    if isinstance(payload, dict):
        return {"fields": {name: _encode_field(value) for name, value in payload.items()}}
    raise TypeError(f"a payload is a str, bytes or a dict of fields, not {type(payload).__name__}")


def _encode_field(value: Any) -> Any:
    if isinstance(value, (bytes, bytearray, memoryview)):
        return {"hex": bytes(value).hex()}
    return value


def _decode_field(value: Any) -> Any:
    if isinstance(value, dict) and set(value) == {"hex"}:
        return bytes.fromhex(value["hex"])
    return value


def _decode_payload(payload: Optional[Dict[str, Any]]) -> Union[Payload, None]:
    if payload is None:
        return None
    if "text" in payload:
        return payload["text"]
    if "hex" in payload:
        return bytes.fromhex(payload["hex"])
    return {name: _decode_field(value) for name, value in payload["fields"].items()}


@dataclass(frozen=True)
class Unit:
    id: str
    type: str
    #: ``"active"``, ``"archived"`` or ``"deleted"``.
    lifecycle: str
    revision: int
    created_at_us: int
    updated_at_us: int
    #: ``None`` for a deleted unit.
    payload: Union[Payload, None]

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "Unit":
        return cls(
            id=data["id"],
            type=data["type"],
            lifecycle=data["lifecycle"],
            revision=data["revision"],
            created_at_us=data["created_at_us"],
            updated_at_us=data["updated_at_us"],
            payload=_decode_payload(data["payload"]),
        )


@dataclass(frozen=True)
class Page:
    """One page of ``list_units``."""

    units: List[Unit]
    #: Pass it as ``after`` to get the next page; ``None`` on the last one.
    next: Optional[str]


@dataclass(frozen=True)
class Relation:
    id: str
    from_id: str
    to_id: str
    type: str
    provenance: str
    #: Only in a list: ``"out"`` when the listed unit is ``from_id``, ``"in"`` when it is ``to_id``.
    direction: Optional[str] = None

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "Relation":
        return cls(
            id=data["id"],
            from_id=data["from"],
            to_id=data["to"],
            type=data["type"],
            provenance=data["provenance"],
            direction=data.get("direction"),
        )


@dataclass(frozen=True)
class FieldDef:
    """A field of a schema. ``type`` is ``bool``, ``int``, ``float``, ``text`` or ``bytes``."""

    name: str
    type: str
    required: bool = False
    #: No two active units hold the same value. Implies ``indexed``.
    unique: bool = False
    indexed: bool = False

    def to_json(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "type": self.type,
            "required": self.required,
            "unique": self.unique,
            "indexed": self.indexed,
        }

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "FieldDef":
        return cls(data["name"], data["type"], data["required"], data["unique"], data["indexed"])


@dataclass(frozen=True)
class Schema:
    type: str
    fields: List[FieldDef]

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "Schema":
        return cls(data["type"], [FieldDef.from_json(f) for f in data["fields"]])


@dataclass(frozen=True)
class Access:
    """How the first binding of a query is read: ``"type_scan"`` or ``"field_index"``."""

    kind: str
    #: The indexed field, for ``"field_index"``.
    field: Optional[str] = None

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "Access":
        return cls(data["kind"], data.get("field"))


@dataclass(frozen=True)
class QueryResult:
    aliases: List[str]
    #: One unit per alias in each row.
    rows: List[List[Unit]]
    access: Access

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "QueryResult":
        rows = [[Unit.from_json(u) for u in row] for row in data["rows"]]
        return cls(data["aliases"], rows, Access.from_json(data["access"]))


@dataclass(frozen=True)
class Candidate:
    access: Access
    cost: int


@dataclass(frozen=True)
class JoinStep:
    alias: str
    unit_type: str
    relation_type: str
    direction: str
    from_alias: str


@dataclass(frozen=True)
class Plan:
    root_alias: str
    root_type: str
    candidates: List[Candidate]
    #: Index into ``candidates`` of the access path chosen.
    chosen: int
    joins: List[JoinStep]

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "Plan":
        return cls(
            data["root_alias"],
            data["root_type"],
            [Candidate(Access.from_json(c["access"]), c["cost"]) for c in data["candidates"]],
            data["chosen"],
            [
                JoinStep(j["alias"], j["unit_type"], j["relation_type"], j["direction"], j["from"])
                for j in data["joins"]
            ],
        )


# ------------------------------------------------------------------ search


@dataclass(frozen=True)
class VectorLeg:
    """The vector leg of a search: the server keeps no vector index, so the request carries the
    vectors of the units to rank. ``metric`` is ``"cosine"`` or ``"dot"``."""

    query: List[float]
    vectors: Dict[str, List[float]]
    metric: Optional[str] = None

    def to_json(self) -> Dict[str, Any]:
        return _drop_none({"query": self.query, "vectors": self.vectors, "metric": self.metric})


@dataclass(frozen=True)
class GraphLeg:
    """The graph leg: units reached by following relations from ``seeds``. ``direction`` is
    ``"out"``, ``"in"`` or ``"both"``; a bound left ``None`` takes the server's default."""

    seeds: List[str]
    direction: Optional[str] = None
    relation_types: Optional[List[str]] = None
    max_depth: Optional[int] = None
    limit: Optional[int] = None
    visit_budget: Optional[int] = None

    def to_json(self) -> Dict[str, Any]:
        return _drop_none(
            {
                "seeds": self.seeds,
                "direction": self.direction,
                "relation_types": self.relation_types,
                "max_depth": self.max_depth,
                "limit": self.limit,
                "visit_budget": self.visit_budget,
            }
        )


@dataclass(frozen=True)
class Weights:
    """How much each leg counts; 0 switches a leg off, ``None`` is the server's default (1)."""

    keyword: Optional[int] = None
    vector: Optional[int] = None
    graph: Optional[int] = None

    def to_json(self) -> Dict[str, Any]:
        return _drop_none({"keyword": self.keyword, "vector": self.vector, "graph": self.graph})


@dataclass(frozen=True)
class Ranks:
    """The 1-based rank a unit held in each leg that found it."""

    keyword: Optional[int]
    vector: Optional[int]
    graph: Optional[int]


@dataclass(frozen=True)
class Hit:
    id: str
    #: The sum of what each leg earned, in units of 1/1e9 (see ``kerno_units::hybrid``).
    score: int
    ranks: Ranks
    unit: Unit

    @classmethod
    def from_json(cls, data: Dict[str, Any]) -> "Hit":
        ranks = data["ranks"]
        return cls(
            data["id"],
            data["score"],
            Ranks(ranks["keyword"], ranks["vector"], ranks["graph"]),
            Unit.from_json(data["unit"]),
        )


@dataclass(frozen=True)
class Health:
    status: str
    version: str


__all__ = [
    "Access", "Candidate", "FieldDef", "Fields", "GraphLeg", "Health", "Hit", "JoinStep", "Page",
    "Payload", "Plan", "QueryResult", "Ranks", "Relation", "Schema", "Unit", "VectorLeg", "Weights",
    "encode_payload",
]
