"""What a generated handle reaches its engine through (ADR 0099).

The raw call, the ID its calls carry, and the host of each handle it opens
or builds, for the async handles and their sync twins. Hand-written, since
no catalog entry shapes it; the client package's runtime implements it.
TypeScript's `handle.ts` is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Protocol, TypeVar

if TYPE_CHECKING:
    from pydantic import BaseModel

_Id = TypeVar("_Id", covariant=True)
_ChildId = TypeVar("_ChildId")


class AsyncHandleHost(Protocol[_Id]):
    """What an async handle of ID `_Id` reaches its engine through."""

    @property
    def id(self) -> _Id:
        """The ID each of the handle's calls carries."""
        ...

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...

    def child(self, id: _ChildId) -> AsyncHandleHost[_ChildId]:
        """The host of a handle this one opens or builds, whose calls carry `id`."""
        ...

    async def close(self, operation: str, params: BaseModel) -> None:
        """Closes the handle through `operation`, its own `close` (ADR 0078).

        Idempotent: it is sent once, and nothing is sent for a handle already
        closed, by its program or with the value that opened it. Every other
        call on the handle, and on a handle it opened or built, then fails
        with `closed`.
        """
        ...


class HandleHost(Protocol[_Id]):
    """What a sync handle of ID `_Id` reaches its engine through."""

    @property
    def id(self) -> _Id:
        """The ID each of the handle's calls carries."""
        ...

    def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...

    def child(self, id: _ChildId) -> HandleHost[_ChildId]:
        """The host of a handle this one opens or builds, whose calls carry `id`."""
        ...

    def close(self, operation: str, params: BaseModel) -> None:
        """Closes the handle through `operation`, its own `close` (ADR 0078).

        Idempotent, as `AsyncHandleHost.close` is.
        """
        ...
