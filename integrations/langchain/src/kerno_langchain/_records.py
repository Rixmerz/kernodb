"""What the vector store keeps in Kerno, and the helpers to read it back.

One document is one typed unit of the store's collection (a unit type). The same helpers are in
``integrations/llamaindex``: the two packages do not depend on each other.
"""

from __future__ import annotations

import math
import struct
from typing import List, Sequence

from kerno_client import ApiError, FieldDef, KernoClient, Unit


def quote(text: str) -> str:
    """``text`` as a string literal of the query language (``docs/query-language.md``)."""
    escaped = text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
    escaped = escaped.replace("\r", "\\r").replace("\t", "\\t").replace("\0", "\\0")
    return f'"{escaped}"'


def pack(vector: Sequence[float]) -> bytes:
    """Little-endian float64, so the vector reads back exactly."""
    return struct.pack(f"<{len(vector)}d", *vector)


def unpack(raw: bytes) -> List[float]:
    return list(struct.unpack(f"<{len(raw) // 8}d", raw))


def cosine(a: Sequence[float], b: Sequence[float]) -> float:
    """The cosine similarity; 0 when either vector is all zeros."""
    if len(a) != len(b):
        raise ValueError(f"vectors of different sizes: {len(a)} and {len(b)}")
    norm = math.sqrt(sum(x * x for x in a)) * math.sqrt(sum(y * y for y in b))
    return sum(x * y for x, y in zip(a, b)) / norm if norm else 0.0


def ensure_schema(kerno: KernoClient, unit_type: str, fields: Sequence[FieldDef]) -> None:
    """Gives ``unit_type`` its schema, or checks that it already has this one.

    A type has one schema for good, so a second store on the same collection, a restart and two
    processes starting together must all find the schema they would have written.
    """
    want = sorted(
        (FieldDef(f.name, f.type, f.required, f.unique, f.indexed or f.unique) for f in fields),
        key=lambda f: f.name,
    )
    try:
        have = kerno.get_schema(unit_type)
    except ApiError as e:
        if e.status != 404:
            raise
        try:
            have = kerno.define_schema(unit_type, fields)
        except ApiError as define_error:
            try:
                have = kerno.get_schema(unit_type)  # defined by another process in between
            except ApiError:
                raise define_error from None
    if sorted(have.fields, key=lambda f: f.name) != want:
        raise ValueError(f"unit type {unit_type!r} already has a schema that is not this store's: use another collection")


def scan(kerno: KernoClient, unit_type: str, where: str = "") -> List[Unit]:
    """Every active unit of the type, in creation order (ids are UUIDv7)."""
    rows = kerno.query(f"FIND {quote(unit_type)} AS d {where}").rows
    return sorted((row[0] for row in rows), key=lambda u: u.id)


def where_equals(field: str, value: str) -> str:
    return f"WHERE d.{field} = {quote(value)}"
