"""The base of every error a Goodfetch SDK raises (ADR 0079).

Each Error code's generated class extends it through its group's.
Hand-written: only the seven groups come from the catalog.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.errors import ErrorGroup


class GoodfetchError(Exception):
    """The base of every error a Goodfetch SDK raises (ADR 0079)."""

    code: str
    """The Error code, as the engine sent it."""
    group: ErrorGroup
    """The code's group, named for its remedy."""
    retryable: bool
    """Whether the same call may succeed when made again."""

    def __init__(
        self, message: str, *, code: str, group: ErrorGroup, retryable: bool
    ) -> None:
        """Takes the message and the code's fields."""
        super().__init__(message)
        self.code = code
        self.group = group
        self.retryable = retryable
