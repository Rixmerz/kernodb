from os import PathLike
from types import TracebackType
from typing import Literal

__all__ = ["Database", "KernoError", "Relation", "Revision", "Unit", "open"]

class KernoError(Exception):
    """Any failure reported by the Kerno engine."""

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
    def relate(self, from_id: str, to_id: str, relation_type: str) -> str:
        """Relate two units; returns the relation's id."""
    def related(
        self,
        id: str,
        direction: Literal["out", "in"] = "out",
        relation_type: str | None = None,
    ) -> list[Relation]:
        """Relations that start at (`"out"`) or end at (`"in"`) the unit."""
    def query(self, unit_type: str | None = None, schema_version: int | None = None) -> list[Unit]:
        """Every active unit, in id order, optionally narrowed by type and schema version."""
    def update(self, id: str, payload: str | bytes) -> None:
        """Replace the payload of `id`, adding a revision."""
    def delete(self, id: str) -> None:
        """Delete `id` with a tombstone revision; the history is kept."""
    def unrelate(self, relation_id: str) -> None:
        """Remove a relation."""
    def commit(self, ops: list[tuple[object, ...]]) -> list[str]:
        """Commit ("store", type, payload) / ("update", id, payload) / ("delete", id) /
        ("unrelate", relation_id) ops atomically; returns the stored ids in order."""
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

def open(path: str | PathLike[str]) -> Database:
    """Open the database in `path`, creating it when `path` holds none."""
