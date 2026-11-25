"""Kerno: an embedded database of units, the relations between them, and queries over both."""

import functools as _functools
from importlib import metadata as _metadata
from typing import Any as _Any

from kerno import _kerno
from kerno._kerno import (
    BadEndpointError,
    ChangeSetTooLargeError,
    ClosedError,
    Comparison,
    DatabaseNotFoundError,
    HybridHit,
    CorruptionError,
    Database,
    DeletedError,
    HasRelationsError,
    HistoryUnavailableError,
    InvalidArgumentError,
    KernoError,
    LockedError,
    NotFoundError,
    NotIndexedError,
    NotUniqueError,
    PayloadTooLargeError,
    PrunedError,
    ReadOnlyError,
    SchemaError,
    SchemaExistsError,
    Relation,
    Revision,
    TextHit,
    Unit,
    UnknownTypeError,
)

try:
    __version__ = _metadata.version("kernodb")
except _metadata.PackageNotFoundError:  # pragma: no cover - a source tree that was never installed
    __version__ = "0+unknown"


def _typed(fn: _Any) -> _Any:
    """Turn a wrong argument type (`None`, a list for a string, ...) or a string that is not
    UTF-8 (a lone surrogate) into `InvalidArgumentError`: no call lets a bare `TypeError` or
    `UnicodeEncodeError` out for a value the caller passed."""

    @_functools.wraps(fn)
    def call(*args: _Any, **kwargs: _Any) -> _Any:
        try:
            return fn(*args, **kwargs)
        except (TypeError, UnicodeEncodeError) as e:
            raise InvalidArgumentError(str(e)) from e

    return call


for _name in dir(Database):
    if not _name.startswith("_"):
        setattr(Database, _name, _typed(getattr(Database, _name)))
open = _typed(_kerno.open)  # noqa: A001 - the documented entry point

__all__ = [
    "__version__",
    "BadEndpointError",
    "ChangeSetTooLargeError",
    "ClosedError",
    "Comparison",
    "DatabaseNotFoundError",
    "HybridHit",
    "CorruptionError",
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
    "Relation",
    "Revision",
    "TextHit",
    "Unit",
    "UnknownTypeError",
    "open",
]
