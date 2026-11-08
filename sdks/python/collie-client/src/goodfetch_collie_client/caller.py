"""What sends a call and reads its result (ADR 0038).

The interfaces every Product's generated operations take, which the client
package's runtime implements. Hand-written, since no catalog entry shapes them.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Protocol, cast

if TYPE_CHECKING:
    from pydantic import BaseModel


class AsyncCaller(Protocol):
    """What sends a call for the async methods: the client package's runtime."""

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...


class Caller(Protocol):
    """What sends a call for the sync methods: the client package's runtime."""

    def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...


def fields_of(result: object) -> dict[str, Any]:
    """The fields of a result, the JSON object the engine sent.

    Read without validation, so a field or value a newer engine added never
    raises (ADR 0082).
    """
    return cast("dict[str, Any]", result)
