"""A Connection belongs to the process that opened it (ADR 0078).

A child created by `fork` inherits a copy of its parent's Connection and of
every Run, Session and page under it, whose pipe, engine and private thread
stay the parent's: each use in the child fails with `connection_forked`
before it writes anything, and the child connects on its own. No fork hook
is installed (ADR 0078): each use compares the process ID with the
opener's.
"""

from __future__ import annotations

import os

from goodfetch_collie_client.generated.errors import ConnectionForkedError

_FIX = (
    "Connect in the child process: a Connection and every Run, Session or"
    " page under it belong to the process that opened them."
)


class OpeningProcess:
    """The process that opened a Connection, the only one that may use it."""

    def __init__(self) -> None:
        """Takes the calling process for the opener."""
        self._pid = os.getpid()

    def is_current(self) -> bool:
        """Whether the calling process is the opener."""
        return os.getpid() == self._pid

    def ensure_current(self, operation: str | None = None) -> None:
        """Raises `connection_forked` unless the calling process is the opener.

        Raises:
            ConnectionForkedError: Another process, such as a forked child,
                made the call.
        """
        if self.is_current():
            return
        message = (
            "this Connection was opened by another process, such as the parent"
            " of a fork; connect in the child process"
        )
        raise ConnectionForkedError(
            message if operation is None else f"{operation}: {message}",
            operation=operation,
            fix=_FIX,
        )
