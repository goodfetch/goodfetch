"""A close every caller shares: the first runs it, the others wait for it."""

from __future__ import annotations

from typing import TYPE_CHECKING

import anyio

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable


class Once:
    """One close of a value, whose failure each caller of it raises."""

    def __init__(self) -> None:
        """Starts with no close begun."""
        self._done: anyio.Event | None = None
        self._failure: Exception | None = None

    @property
    def started(self) -> bool:
        """Whether a close has begun."""
        return self._done is not None

    async def run(
        self, close: Callable[[], Awaitable[None]], *, shield: bool = False
    ) -> None:
        """Runs `close` the first time, or waits for the close begun.

        With `shield`, cancelling the first caller does not cut `close` short.
        """
        if self._done is not None:
            await self._done.wait()
        else:
            self._done = anyio.Event()
            try:
                with anyio.CancelScope(shield=shield):
                    await close()
            except Exception as failure:
                self._failure = failure
            finally:
                self._done.set()
        if self._failure is not None:
            raise self._failure
