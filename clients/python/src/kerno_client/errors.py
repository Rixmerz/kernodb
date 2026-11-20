"""What a call can raise."""

from __future__ import annotations

from typing import Optional


class KernoError(Exception):
    """Base class of every error this package raises."""


class TransportError(KernoError):
    """The server could not be reached, or did not answer in time."""


class ApiError(KernoError):
    """The server answered with an error.

    ``status`` is the HTTP status and ``code`` the machine-readable error code of the server
    (``not_found``, ``revision_conflict``, ``schema_violation``, ...; see the kerno-server README).
    ``code`` is ``None`` when the answer was not a kerno-server error body (a proxy's page, say).
    """

    def __init__(self, status: int, code: Optional[str], message: str) -> None:
        super().__init__(f"{status} {code or 'error'}: {message}")
        self.status = status
        self.code = code
        self.message = message
