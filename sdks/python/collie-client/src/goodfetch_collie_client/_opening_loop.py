"""An async Connection belongs to the event loop that opened it (ADR 0081).

Its channel's locks, streams and tasks are that loop's, and so are those of
every Run, Session and page under it: a call from another loop, on another
thread, would fail later with "the future belongs to a different loop", or
hang. Each use compares the running loop with the opener's instead, and
fails with `event_loop_mismatch` before it sends anything. The sync client
runs its Connection on a private loop of its own, so its values may be
called from any thread.
"""

from __future__ import annotations

from typing import assert_never

import anyio
import anyio.lowlevel

from goodfetch_collie_client.generated.errors import EventLoopMismatchError
from goodfetch_collie_client.generated.models import Product
from goodfetch_collie_client.goodfetch_error import runtime_message


def _sync_client(needs: Product) -> str:
    """The sync client class of the Product `needs`, which the fix names."""
    match needs:
        case Product.collie:
            return "Collie"
        case Product.foxhound:
            return "Foxhound"
        case _:
            assert_never(needs)


def _running_loop() -> object | None:
    """The event loop running the caller, none on a thread without one."""
    try:
        return anyio.lowlevel.current_token().native_token
    except anyio.NoEventLoopError:
        return None


class OpeningLoop:
    """The event loop that opened a Connection, the only one that may use it."""

    def __init__(self, needs: Product) -> None:
        """Takes the running loop for the opener, of a Connection to `needs`.

        Raises:
            anyio.NoEventLoopError: No event loop runs the caller.
        """
        self._loop = anyio.lowlevel.current_token().native_token
        self._fix = (
            "Call it from the event loop that opened its Connection, or use"
            f" the sync `{_sync_client(needs)}`, whose values any thread may"
            " call."
        )

    def ensure_current(self, operation: str | None = None) -> None:
        """Raises `event_loop_mismatch` unless the opener's loop runs the caller.

        Raises:
            EventLoopMismatchError: Another event loop, or a thread running
                none, made the call.
        """
        if _running_loop() is self._loop:
            return
        message = runtime_message(
            "this async value belongs to the event loop that opened its"
            " Connection, and another event loop or thread called it",
            operation=operation,
            fix=self._fix,
        )
        raise EventLoopMismatchError(message, operation=operation, fix=self._fix)
