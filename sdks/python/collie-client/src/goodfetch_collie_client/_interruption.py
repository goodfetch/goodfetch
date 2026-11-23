"""A sync call's interruption, heard on the loop before its cancellation lands.

An exception raised in the thread blocked in a sync call, as Ctrl+C's
`KeyboardInterrupt`, cancels the call and returns to the caller at once
(ADR 0081), while the call's task may notice its cancellation only later.
The private loop (`portal`) runs each sync call under an `Interruption`,
which the call's `Channel` listens to, so that on the loop, before the
caller's thread goes on, the Channel puts the call's `$/cancel` ahead of
any later request (issue 3517). A call of the async API runs under none:
its cancellation reaches its caller only once its `$/cancel` is written.
"""

from __future__ import annotations

from contextvars import ContextVar
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Callable

__all__ = ["Interruption", "when_interrupted"]


def _nothing() -> None:
    return None


class Interruption:
    """One sync call's interruption, which its caller's thread signals on the loop."""

    def __init__(self) -> None:
        """Starts uninterrupted, with no action waiting for it."""
        self._actions: list[Callable[[], None]] = []
        self._interrupted = False

    def govern(self) -> None:
        """Makes it the interruption of the calls the current context makes.

        The private loop runs each sync call in a context of its own, a copy
        of its caller's thread's, so it is set there alone.
        """
        _CURRENT.set(self)

    def interrupt(self) -> None:
        """Runs each action waiting, on the loop, and any added later at once."""
        self._interrupted = True
        for action in list(self._actions):
            action()

    def add(self, action: Callable[[], None]) -> Callable[[], None]:
        """Runs `action` once it is interrupted, at once if it was; what stops it."""
        if self._interrupted:
            action()
            return _nothing
        self._actions.append(action)
        return lambda: self._actions.remove(action)


_CURRENT: ContextVar[Interruption | None] = ContextVar(
    "goodfetch_interruption", default=None
)
"""The interruption of the sync call the current context runs, if any."""


def when_interrupted(action: Callable[[], None]) -> Callable[[], None]:
    """Runs `action` on the loop once the current sync call is interrupted.

    It runs at once when the call was interrupted already, and never for a
    call of the async API. `action` must not block.

    Returns:
        What stops `action` from running, once the call no longer needs it.
    """
    interruption = _CURRENT.get()
    if interruption is None:
        return _nothing
    return interruption.add(action)
