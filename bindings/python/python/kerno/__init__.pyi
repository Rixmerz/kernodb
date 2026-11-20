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
    def query(self, unit_type: str | None = None, schema_version: int | None = None) -> list[Unit]:
        """Every active unit, in id order, optionally narrowed by type and schema version."""
    def update(self, id: str, payload: str | bytes, if_changed: bool = False) -> bool:
        """Replace the payload of `id`; returns whether a revision was written."""
    def delete(self, id: str, cascade: bool = False) -> None:
        """Delete `id` with a tombstone revision (and its relations with `cascade`)."""
    def relations(self, relation_type: str | None = None) -> list[Relation]:
        """Every live relation (optionally of one type), in one scan."""
    def last_commit_seq(self) -> int: ...
    def compact(self) -> dict[str, int]: ...
    def define_schema(self, unit_type: str, fields: list[dict[str, object]]) -> None: ...
    def create_index(self, unit_type: str, field: str) -> None: ...
    def store_document(self, unit_type: str, document: dict[str, FieldValue]) -> str: ...
    def update_document(self, id: str, document: dict[str, FieldValue]) -> None: ...
    def read_document(self, id: str) -> dict[str, FieldValue] | None: ...
    def find(self, unit_type: str, field: str, value: FieldValue) -> list[Unit]: ...
    def execute(self, query: str) -> dict[str, object]: ...
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
    def history(self, id: str, limit: int = 100, with_payload: bool = False) -> list[Revision]:
        """The newest `limit` revisions of `id`, newest first."""
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

def open(path: str | PathLike[str], read_only: bool = False) -> Database:
    """Open the database in `path`, creating it when `path` holds none."""
