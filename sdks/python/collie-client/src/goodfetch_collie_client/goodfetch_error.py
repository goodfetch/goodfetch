"""The base of every error a Goodfetch SDK raises (ADR 0079).

Each Error code's generated class extends it through its group's.
Hand-written: only the seven groups come from the catalog.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Protocol

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client.generated.errors import ErrorGroup


class GoodfetchError(Exception):
    """The base of every error a Goodfetch SDK raises (ADR 0079)."""

    code: str
    """The Error code, as the engine sent it."""
    group: ErrorGroup
    """The code's group, named for its remedy."""
    retryable: bool
    """Whether the same call may succeed when made again."""
    fix: str | None
    """What to do about it, when the engine or the runtime knows."""
    details: Mapping[str, object]
    """The code's details, in wire names; empty when it has none."""

    def __init__(
        self,
        message: str,
        *,
        code: str,
        group: ErrorGroup,
        retryable: bool,
        fix: str | None = None,
        details: Mapping[str, object] | None = None,
    ) -> None:
        """Takes the message, the code's fields and what the error carries."""
        super().__init__(message)
        self.code = code
        self.group = group
        self.retryable = retryable
        self.fix = fix
        self.details = {} if details is None else dict(details)


class ErrorClass(Protocol):
    """The class of one Error code: its message, then what it carries."""

    def __call__(
        self,
        message: str,
        *,
        fix: str | None = None,
        details: Mapping[str, object] | None = None,
    ) -> GoodfetchError:
        """Makes the error from the message the engine rendered."""
        ...
