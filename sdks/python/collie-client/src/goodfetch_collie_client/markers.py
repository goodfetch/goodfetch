"""ADR 0030's run-time warning for a deprecated or Experimental catalog entry.

Each generated member of such an entry calls it, and each hand-written member
serving one, or listener of an event, reads its entry's markers from the
generated registries through `warn_entry` (ADR 0099). It warns once per
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
from collections.abc import Mapping
from typing import TYPE_CHECKING, Final, cast

if TYPE_CHECKING:
    from collections.abc import Set as AbstractSet

# The frames between `warnings.warn` and the member's caller: `_warn_once`,
# `warn_deprecated`, `warn_experimental`, `warn_entry` or
# `warn_deprecated_settings`, then the member.
_CALLER: Final = 4


class ExperimentalWarning(UserWarning):
    """A member outside semver until it is promoted was used (ADR 0030)."""


_warned: Final[set[tuple[str, str]]] = set()
_lock: Final = threading.Lock()


def warn_deprecated(name: str, text: str) -> None:
    """Warns, the first time in the process, that the entry `name` is deprecated.

    `text` is its deprecation's one text, as its `@deprecated` renders it.
    """
    _warn_once(("deprecated", name), _deprecated(name, text), DeprecationWarning)


def warn_experimental(name: str) -> None:
    """Warns, the first time in the process, that the entry `name` is Experimental."""
    _warn_once(("experimental", name), _experimental(name), ExperimentalWarning)


def warn_entry(
    name: str, experimental: AbstractSet[str], deprecated: Mapping[str, str]
) -> None:
    """Warns, the first time in the process, of each marker the entry `name` carries.

    `experimental` and `deprecated` are the generated registries of its
    section, such as `EXPERIMENTAL_OPERATIONS` and `DEPRECATED_OPERATIONS`:
    a hand-written member serving the entry, or a listener of an event,
    calls it first, as a generated member calls `warn_experimental` and
    `warn_deprecated`.
    """
    if name in experimental:
        _warn_once(("experimental", name), _experimental(name), ExperimentalWarning)
    text = deprecated.get(name)
    if text is not None:
        _warn_once(("deprecated", name), _deprecated(name, text), DeprecationWarning)


def warn_deprecated_settings(
    settings: Mapping[str, object] | None, deprecated: Mapping[str, str]
) -> None:
    """Warns, the first time in the process, of each deprecated setting `settings` sets.

    `deprecated` is the generated `DEPRECATED_SETTINGS`, each deprecated
    setting's text by its dotted wire key: a member taking `settings`
    calls it first, and a key set to `None`, which clears, is set.
    """
    if settings is None:
        return
    for key, text in deprecated.items():
        if _sets(settings, key):
            _warn_once(
                ("deprecated setting", key),
                f"The setting `{key}` is deprecated. {text}",
                DeprecationWarning,
            )


def _sets(settings: Mapping[str, object], key: str) -> bool:
    """Whether `settings`, nested by segment, set the dotted `key`."""
    *areas, last = key.split(".")
    level = settings
    for segment in areas:
        inner = level.get(segment)
        if not isinstance(inner, Mapping):
            return False
        level = cast("Mapping[str, object]", inner)
    return last in level


def _deprecated(name: str, text: str) -> str:
    return f"`{name}` is deprecated. {text}"


def _experimental(name: str) -> str:
    return f"`{name}` is Experimental: outside semver until it is promoted (ADR 0030)."


def _warn_once(key: tuple[str, str], message: str, category: type[Warning]) -> None:
    with _lock:
        if key in _warned:
            return
        _warned.add(key)
    warnings.warn(message, category, stacklevel=_CALLER)
