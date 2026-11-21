from os import PathLike
from types import TracebackType
from typing import Literal

__all__ = [
    "CorruptionError",
    "Database",
    "DeletedError",
    "HasRelationsError",
    "InvalidArgumentError",
    "KernoError",
    "LockedError",
    "NotFoundError",
    "PayloadTooLargeError",
    "PrunedError",
    "SchemaError",
    "Comparison",
    "HybridHit",
    "Relation",
    "Revision",
    "Unit",
    "open",
]

class KernoError(Exception):
    """Any failure reported by the Kerno engine."""

class NotFoundError(KernoError): ...
class DeletedError(KernoError): ...
class LockedError(KernoError): ...
class CorruptionError(KernoError): ...
class InvalidArgumentError(KernoError): ...
class PayloadTooLargeError(KernoError): ...
class HasRelationsError(KernoError): ...
class PrunedError(KernoError): ...
class SchemaError(KernoError): ...

FieldValue = bool | int | float | str | bytes

class Unit:
    """A stored unit, as of its latest revision."""

    @property
    def id(self) -> str: ...
    @property
    def unit_type(self) -> str: ...
    @property
    def schema_version(self) -> int: ...
    @property
    def lifecycle(self) -> Literal["active", "archived", "deleted"]: ...
    @property
    def revision(self) -> int: ...
    @property
    def oldest_revision(self) -> int:
        """The oldest revision retention still keeps (1 until it prunes the unit)."""
    @property
    def payload(self) -> bytes | None:
        """The content; `None` for a deleted unit."""

class Relation:
    """A directed, typed edge between two units."""

    @property
    def id(self) -> str: ...
    @property
    def from_id(self) -> str: ...
    @property
    def to_id(self) -> str: ...
    @property
    def relation_type(self) -> str: ...

class Revision:
    """One revision of a unit, from `history` or `read_as_of`."""

    @property
    def revision(self) -> int: ...
    @property
    def kind(self) -> Literal["create", "update", "archive", "unarchive", "delete", "restore"]: ...
    @property
    def commit_seq(self) -> int: ...
    @property
    def created_at(self) -> int: ...
    @property
    def lifecycle(self) -> Literal["active", "archived", "deleted"]: ...
    @property
    def schema_version(self) -> int: ...
    @property
    def content_hash(self) -> str: ...
    @property
    def payload(self) -> bytes | None:
        """The verified content, when asked for and present."""

class Comparison:
    """Two revisions of one unit, from `compare`."""

    @property
    def a(self) -> Revision: ...
    @property
    def b(self) -> Revision: ...
    @property
    def same_content(self) -> bool: ...

class HybridHit:
    """One hit of `hybrid_search`; a signal's score and rank are `None` when it missed the unit."""

    @property
    def id(self) -> str: ...
    @property
    def score(self) -> float:
        """The sum of the signal scores."""
    @property
    def keyword_score(self) -> float | None: ...
    @property
    def vector_score(self) -> float | None: ...
    @property
    def relation_score(self) -> float | None: ...
    @property
    def keyword_rank(self) -> int | None: ...
    @property
    def vector_rank(self) -> int | None: ...
    @property
    def relation_rank(self) -> int | None: ...

class Database:
    """An open Kerno database. Use it as a context manager, or call `close()`."""

    def store(self, unit_type: str, payload: str | bytes) -> str:
        """Store a new unit and return its id. A `str` payload is stored as UTF-8."""
    def read(self, id: str) -> Unit | None:
        """The unit `id`, or `None` when no such unit exists."""
    def relate(self, from_id: str, to_id: str, relation_type: str, unique: bool = False) -> str:
        """Relate two units; returns the relation's id. `unique` reuses an existing one."""
    def related(
        self,
        id: str,
        direction: Literal["out", "in"] = "out",
        relation_type: str | None = None,
    ) -> list[Relation]:
        """Relations that start at (`"out"`) or end at (`"in"`) the unit."""
    def query(
        self,
        unit_type: str | None = None,
        schema_version: int | None = None,
        lifecycle: Literal["active", "archived", "deleted", "all"] = "active",
    ) -> list[Unit]:
        """Units in id order, narrowed by type, schema version and lifecycle (active by default)."""
    def update(self, id: str, payload: str | bytes, if_changed: bool = False) -> bool:
        """Replace the payload of `id`; returns whether a revision was written."""
    def delete(self, id: str, cascade: bool = False) -> None:
        """Delete `id` with a tombstone revision (and its relations with `cascade`)."""
    def relations(self, relation_type: str | None = None) -> list[Relation]:
        """Every live relation (optionally of one type), in one scan."""
    def fulltext(self, unit_type: str) -> bool:
        """Whether the type is keyword-indexed (off by default)."""
    def set_fulltext(self, unit_type: str, enabled: bool) -> None:
        """Keyword-index a type or not (off by default); its units follow in the same commit."""
    def last_commit_seq(self) -> int: ...
    def compact(self) -> dict[str, int]: ...
    def define_schema(self, unit_type: str, fields: list[dict[str, object]]) -> None: ...
    def create_index(self, unit_type: str, field: str) -> None: ...
    def store_document(self, unit_type: str, document: dict[str, FieldValue]) -> str: ...
    def update_document(self, id: str, document: dict[str, FieldValue]) -> None: ...
    def read_document(self, id: str) -> dict[str, FieldValue] | None: ...
    def find(self, unit_type: str, field: str, value: FieldValue) -> list[Unit]: ...
    def find_range(
        self,
        unit_type: str,
        field: str,
        low: FieldValue | None = None,
        high: FieldValue | None = None,
        limit: int | None = None,
    ) -> list[Unit]:
        """Active units whose indexed field is in `[low, high]` (`None` = open end), by value."""
    def execute(self, query: str) -> dict[str, object]: ...
    def explain(self, query: str) -> dict[str, object]:
        """How `execute` would run `query`: candidates with costs, the chosen one, the joins."""
    def hybrid_search(
        self,
        text: str | None = None,
        vector: list[float] | None = None,
        vectors: dict[str, list[float]] | None = None,
        seeds: list[str] | None = None,
        *,
        keyword_weight: int = 1,
        vector_weight: int = 1,
        relation_weight: int = 1,
        limit: int = 10,
        candidates: int = 100,
        rrf_k: int = 60,
        max_depth: int = 2,
        metric: Literal["cosine", "dot"] = "cosine",
    ) -> list[HybridHit]:
        """Fuse keyword (BM25), vector and relation signals by reciprocal rank fusion."""
    def unrelate(self, relation_id: str) -> None:
        """Remove a relation."""
    def commit(
        self,
        ops: list[tuple[object, ...]],
        reason: str | None = None,
        actor: str | bytes | None = None,
        source: str | bytes | None = None,
        detailed: bool = False,
    ) -> list[str] | dict[str, object]:
        """Commit ops atomically (see the README for the op list)."""
    def history(
        self,
        id: str,
        limit: int = 100,
        with_payload: bool = False,
        from_revision: int | None = None,
        to_revision: int | None = None,
        order: Literal["desc", "asc"] = "desc",
    ) -> list[Revision]:
        """Up to `limit` revisions of `id` in `[from_revision, to_revision)`, newest first by default."""
    def compare(self, id: str, a: int, b: int) -> Comparison:
        """Revisions `a` and `b` of unit `id`, with payloads, and whether their content is equal."""
    def read_as_of(self, id: str, commit_seq: int) -> Revision | None:
        """The revision in force once `commit_seq` committed; `None` before creation."""
    def verify(self) -> dict[str, object]:
        """Integrity check of the units layer (clean, commit_seq, counts, findings)."""
    def close(self) -> None:
        """Close the database; closing twice is harmless."""
    def __enter__(self) -> Database: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> bool: ...

def open(path: str | PathLike[str], read_only: bool = False, max_batch_ops: int | None = None) -> Database:
    """Open the database in `path`, creating it when `path` holds none."""
