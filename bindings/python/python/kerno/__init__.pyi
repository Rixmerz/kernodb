from os import PathLike
from types import TracebackType
from collections.abc import Mapping, Sequence
from typing import Literal

__version__: str
"""The installed package version."""

__all__ = [
    "__version__",
    "BadEndpointError",
    "ChangeSetTooLargeError",
    "ClosedError",
    "CorruptionError",
    "DatabaseNotFoundError",
    "Database",
    "DeletedError",
    "HasRelationsError",
    "HistoryUnavailableError",
    "InvalidArgumentError",
    "KernoError",
    "LockedError",
    "NotFoundError",
    "NotIndexedError",
    "NotUniqueError",
    "PayloadTooLargeError",
    "PrunedError",
    "ReadOnlyError",
    "SchemaError",
    "SchemaExistsError",
    "Comparison",
    "HybridHit",
    "Relation",
    "Revision",
    "TextHit",
    "Unit",
    "UnknownTypeError",
    "open",
]

# Exactly one of `keep_last` (newest N revisions, 1 or more), `keep_for` (seconds) or `keep_since` (commit sequence).
RetentionRule = dict[str, float]

class KernoError(Exception):
    """Any failure reported by the Kerno engine."""

class NotFoundError(KernoError): ...
class DeletedError(KernoError): ...
class LockedError(KernoError): ...
class CorruptionError(KernoError): ...
class InvalidArgumentError(KernoError, ValueError):
    """An argument was refused (a malformed id, a bad type name, a limit out of range, a bad batch op).

    Also a `ValueError`: `except KernoError` and `except ValueError` both catch it."""

class ClosedError(KernoError, ValueError):
    """A call on a closed database handle. Also a `ValueError`."""

class HistoryUnavailableError(KernoError):
    """The content of a revision is no longer stored: `compact()` dropped the payloads of a deleted
    unit's history. Raised by `read_as_of` and `compare`; `history(with_payload=True)` lists such
    revisions with `payload` `None` instead."""

class SchemaExistsError(InvalidArgumentError): ...
class NotIndexedError(InvalidArgumentError): ...
class UnknownTypeError(InvalidArgumentError): ...
class NotUniqueError(InvalidArgumentError): ...
class BadEndpointError(InvalidArgumentError): ...
class PayloadTooLargeError(KernoError): ...
class ChangeSetTooLargeError(PayloadTooLargeError): ...
class ReadOnlyError(KernoError): ...
class DatabaseNotFoundError(KernoError): ...
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
    def created_at_us(self) -> int:
        """When the unit was created, in microseconds since the Unix epoch."""
    @property
    def updated_at_us(self) -> int:
        """When the head revision was written, in microseconds since the Unix epoch."""
    @property
    def payload(self) -> bytes | None:
        """The content; `None` for a deleted unit."""
    def document(self) -> dict[str, FieldValue] | None:
        """The payload decoded as a document, without reading the database again.

        `None` for a deleted unit; `SchemaError` when the payload is not a document."""

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
    def created_at_us(self) -> int:
        """Descriptive wall-clock time in microseconds since the Unix epoch; never used for ordering."""
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

class TextHit:
    """One hit of `search_text`: a unit id and its BM25 score (higher is a better match)."""

    @property
    def id(self) -> str: ...
    @property
    def score(self) -> float: ...

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
        """Relations that start at (`"out"`) or end at (`"in"`) the unit.

        An invalid `relation_type` name (empty, over 64 bytes, with a NUL) raises `InvalidArgumentError`,
        and an id no unit ever had raises `NotFoundError` (a deleted unit is known and has none).
        """
    def neighbors(
        self,
        id: str,
        direction: Literal["out", "in"] = "out",
        relation_type: str | None = None,
        type_prefix: str | None = None,
    ) -> list[tuple[Relation, Unit]]:
        """The relations of `id`, as `related`, each with the unit at its other end, in one call.

        With `type_prefix`, relations whose type does not start with it are skipped before the
        unit at their other end is loaded."""
    def find_documents(
        self, unit_type: str, field: str, value: Any, fields: Sequence[str] | Mapping[str, str] | None = None
    ) -> list[dict[str, Any]]:
        """`find`, with each hit as its document dict.

        With `fields`, each dict holds only those field names (one the document lacks is absent).
        As a mapping `{field_name: key_in_result}`, each dict holds those fields under the mapped
        keys, in the mapping's order."""
    def neighbor_documents(
        self,
        unit_type: str,
        field: str,
        value: Any,
        direction: Literal["out", "in"] = "out",
        type_prefix: str | None = None,
        fields: Sequence[str] | Mapping[str, str] | None = None,
    ) -> list[tuple[str, dict[str, Any]]]:
        """`find`, then `neighbors` of each unit found, as `[(relation_type, other end's document)]`.

        `fields` keeps only those field names in each dict, or renames them when it is a mapping
        `{field_name: key_in_result}`, as in `find_documents`."""
    def query(
        self,
        unit_type: str | None = None,
        schema_version: int | None = None,
        lifecycle: Literal["active", "archived", "deleted", "all"] = "active",
    ) -> list[Unit]:
        """Units in id order, narrowed by type, schema version and lifecycle (active by default).

        An invalid `unit_type` name (empty or over 64 bytes) raises `InvalidArgumentError` for every
        `lifecycle`.
        """
    def update(self, id: str, payload: str | bytes, if_changed: bool = False) -> bool:
        """Replace the payload of `id`; returns whether a revision was written."""
    def delete(self, id: str, cascade: bool = False) -> None:
        """Delete `id` with a tombstone revision (and its relations with `cascade`)."""
    def relations(self, relation_type: str | None = None) -> list[Relation]:
        """Every live relation (optionally of one type), in one scan.

        An invalid `relation_type` name raises `InvalidArgumentError`.
        """
    def fulltext(self, unit_type: str) -> bool:
        """Whether the type is keyword-indexed (off by default)."""
    def set_fulltext(self, unit_type: str, enabled: bool) -> None:
        """Keyword-index a type or not (off by default); its units follow in the same commit."""
    def search_text(self, query: str, limit: int = 10, types: list[str] | None = None) -> list[TextHit]:
        """Keyword (BM25) search, best first, optionally only in the unit types `types`.

        Raises instead of returning `[]` for a search that could never match: `NotIndexedError`
        (no type, or a type in `types`, is keyword-indexed), `UnknownTypeError` (a type nothing ever
        used), `InvalidArgumentError` (`limit < 1`, a query without a word, an empty `types`).
        """
    def substring(self, unit_type: str) -> bool:
        """Whether the type is substring-indexed (off by default)."""
    def set_substring(self, unit_type: str, enabled: bool) -> None:
        """Trigram-index a type for substring search or not (off by default); its units follow in the same commit."""
    def search_substring(
        self, needle: str | bytes, limit: int | None = None, types: list[str] | None = None
    ) -> list[str]:
        """The ids of the units whose payload holds `needle`, in id order, at most `limit` (`None`: all).

        ASCII letters match in either case and every other byte must be equal, like SQL's
        `LIKE '%needle%'`; only the first 256 KiB of a payload are searched. `types=None` searches
        the substring-indexed types, or every type when none is. A type without the trigram index
        is searched by scanning its payloads in parallel, with the same answers. Raises instead of
        returning `[]` for a search that could never match: `UnknownTypeError` (a type nothing
        ever used), `InvalidArgumentError` (`limit < 1`, an empty needle or one over 1024 bytes,
        an empty `types`).
        """
    def last_commit_seq(self) -> int: ...
    def set_read_cache_bytes(self, bytes: int) -> None:
        """Set how many bytes of units already read and checked the handle keeps in memory, to answer
        the same reads again until the next commit (32 MiB by default); 0 turns that off. The cache
        starts empty either way."""
    def refresh(self) -> int:
        """Move a `read_only=True` database to the writer's newest commits; returns the sequence it now reads.

        Raises `InvalidArgumentError` on a read-write handle (it is always current)."""
    def compact(self) -> dict[str, int]:
        """Drop deleted units' payloads and compact; returns the stored bytes `before` (at call time, every
        commit counted) and `after`. `after` can be larger than `before` when there is little to
        reclaim."""
    def run_retention(
        self, default: RetentionRule | None = None, by_type: dict[str, RetentionRule] | None = None
    ) -> dict[str, int | None]:
        """Prune old revisions by rule (`by_type`, else `default`; no rule keeps everything).

        Returns `units_retained`, `revisions_pruned`, `payloads_dropped`, `bytes_logically_freed` and
        `commit_seq` (`None` when nothing was pruned). `history` from a pruned revision and
        `read_as_of` before the oldest kept one raise `PrunedError`. The policy is not stored."""
    def define_schema(self, unit_type: str, fields: list[dict[str, object]]) -> None:
        """Give a type a schema. The same schema again is a no-op; a different one raises `SchemaExistsError`."""
    def create_index(self, unit_type: str, field: str) -> None: ...
    def store_document(self, unit_type: str, document: dict[str, FieldValue]) -> str: ...
    def upsert(self, unit_type: str, field: str, document: dict[str, FieldValue]) -> dict[str, str]:
        """Store `document`, or revise the active unit holding the same value of the unique `field`.

        Returns `{"id": ..., "outcome": "created" | "updated" | "unchanged"}`. Raises
        `NotUniqueError` when `field` is not declared unique and `SchemaError` when the
        document has no value in it."""
    def update_document(self, id: str, document: dict[str, FieldValue]) -> None: ...
    def read_document(self, id: str) -> dict[str, FieldValue] | None: ...
    def find(self, unit_type: str, field: str, value: FieldValue) -> list[Unit]:
        """Active units whose indexed `field` equals `value`. `NotIndexedError` for a field that is not indexed;
        `InvalidArgumentError` for a value over 254 bytes, which no unit can hold."""
    def find_range(
        self,
        unit_type: str,
        field: str,
        low: FieldValue | None = None,
        high: FieldValue | None = None,
        limit: int | None = None,
    ) -> list[Unit]:
        """Active units whose indexed field is in `[low, high]` (`None` = open end), by value.

        `InvalidArgumentError` for `limit=0` or a range no value can be in (`low` above `high`).
        """
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
        text_types: list[str] | None = None,
        keyword_weight: int = 1,
        vector_weight: int = 1,
        relation_weight: int = 1,
        limit: int = 10,
        candidates: int = 100,
        rrf_k: int = 60,
        max_depth: int = 2,
        metric: Literal["cosine", "dot"] = "cosine",
    ) -> list[HybridHit]:
        """Fuse keyword (BM25), vector and relation signals by reciprocal rank fusion.

        `text_types` restricts the keyword signal to those unit types (errors as `search_text`);
        it is an error without `text`.
        """
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
        """Commit ops atomically (see the README for the op list).

        A `relate` end is a unit id or the position in `ops` of an earlier `store` or `upsert` op;
        any other position raises `BadEndpointError` and writes nothing."""
    def history(
        self,
        id: str,
        limit: int = 100,
        with_payload: bool = False,
        from_revision: int | None = None,
        to_revision: int | None = None,
        order: Literal["desc", "asc"] = "desc",
    ) -> list[Revision]:
        """Up to `limit` revisions of `id` in `[from_revision, to_revision)`, newest first by default.

        `NotFoundError` for an id no unit ever had; `InvalidArgumentError` when `from_revision` is
        above `to_revision` (equal bounds are an empty page, the end of a paged read). For a deleted
        unit whose payloads `compact()` dropped, `with_payload=True` lists the revisions with `payload`
        `None` (no error).
        """
    def compare(self, id: str, a: int, b: int) -> Comparison:
        """Revisions `a` and `b` of unit `id`, with payloads, and whether their content is equal.

        Raises `HistoryUnavailableError` for a deleted unit after `compact()`."""
    def read_as_of(self, id: str, commit_seq: int) -> Revision | None:
        """The revision in force once `commit_seq` committed; `None` before creation.

        Raises `PrunedError` below the oldest kept revision and `HistoryUnavailableError` for a
        deleted unit after `compact()`."""
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

def open(
    path: str | PathLike[str],
    read_only: bool = False,
    max_batch_ops: int | None = None,
    verify_payload_on_read: bool = False,
) -> Database:
    """Open the database in `path`, creating it when `path` holds none.

    `read_only=True` takes no lock and opens beside a live writer; it reads the state at open time until
    `refresh()`. Writes raise `ReadOnlyError`; a missing database raises `DatabaseNotFoundError`.
    `verify_payload_on_read=True` makes every read also check each payload against its content hash
    (off by default; `verify()` always checks it)."""
