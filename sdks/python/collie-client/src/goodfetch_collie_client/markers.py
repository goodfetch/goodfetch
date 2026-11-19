"""ADR 0030's run-time warning for a deprecated or Experimental catalog entry.

Each generated member of such an entry calls it (ADR 0099). It warns once per
process per name and marker, through `warnings`, the host's channel (ADR
0078), attributed to the line that used the member, so the host's filters
decide what shows: Python hides a `DeprecationWarning` outside `__main__`
and its tests by default, and `ExperimentalWarning` is a `UserWarning`. The
first use is the one warning, whether a filter shows it or not: a deprecated
member first used from a library module stays silent when `__main__` uses
it later. The
type-level marker, PEP 702's `@deprecated` with no category of its own, and
the docstring line are the member's own.
"""

from __future__ import annotations

import threading
import warnings
from typing import Final

# The frames between `warnings.warn` and the member's caller: `_warn_once`,
# `warn_deprecated` or `warn_experimental`, then the generated member.
_CALLER: Final = 4


class ExperimentalWarning(UserWarning):
    """A member outside semver until it is promoted was used (ADR 0030)."""


_warned: Final[set[tuple[str, str]]] = set()
_lock: Final = threading.Lock()


def warn_deprecated(name: str, text: str) -> None:
    """Warns, the first time in the process, that the entry `name` is deprecated.

    `text` is its deprecation's one text, as its `@deprecated` renders it.
    """
    _warn_once(
        ("deprecated", name), f"`{name}` is deprecated. {text}", DeprecationWarning
    )


def warn_experimental(name: str) -> None:
    """Warns, the first time in the process, that the entry `name` is Experimental."""
    _warn_once(
        ("experimental", name),
        f"`{name}` is Experimental: outside semver until it is promoted (ADR 0030).",
        ExperimentalWarning,
    )


def _warn_once(key: tuple[str, str], message: str, category: type[Warning]) -> None:
    with _lock:
        if key in _warned:
            return
        _warned.add(key)
    warnings.warn(message, category, stacklevel=_CALLER)
