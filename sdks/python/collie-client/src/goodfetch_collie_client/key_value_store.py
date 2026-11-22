"""A Run's Key-value store (ADR 0103): `run.key_value_store`.

Its committed keys are listed by `keys(prefix=…)`, an iterator (ADR 0081)
read with `async for`, or `for` in the sync API, in the order of their
Unicode scalar values. The call takes no writer lock and goes when `keys`
is called. TypeScript's `key-value-store.ts` is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from goodfetch_collie_client._framing import object_members
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)
from goodfetch_collie_client.markers import warn_entry

if TYPE_CHECKING:
    from collections.abc import Callable

    from goodfetch_collie_client.sequence import AsyncSequence, Sequence

__all__ = ["AsyncKeyValueStore", "KeyValueStore"]

KEYS = "key_value_store.keys"
"""The operation `keys` sends."""

KEY = "$/key"
"""The notification that carries one of its keys."""


def keys_params(run_id: str, prefix: str | None) -> dict[str, object]:
    """What `keys` sends for the Run `run_id`: its prefix, left out when `None`."""
    params: dict[str, object] = {"run_id": run_id}
    if prefix is not None:
        params["prefix"] = prefix
    return params


def key_of(method: str, params: object) -> str | None:
    """The key a `$/key` notification carries; `None` for another notification."""
    if method != KEY:
        return None
    members = object_members(params)
    key = None if members is None else members.get("key")
    return key if isinstance(key, str) else None


class AsyncKeyValueStore:
    """A Run's Key-value store, in the async API. A program never constructs one."""

    def __init__(self, keys: Callable[[str | None], AsyncSequence[str]]) -> None:
        """Takes what reads its keys under a prefix."""
        self._keys = keys

    def keys(self, *, prefix: str | None = None) -> AsyncSequence[str]:
        """The committed keys that start with `prefix`, every key when left out.

        Sends the committed keys of a Run's Key-value store that start with
        `prefix`, in the order of their Unicode scalar values, in `$/key`
        notifications, one per credit granted with `$/credit`, then answers how many
        it sent. Takes no writer lock.

        Experimental: outside semver until it is promoted (ADR 0030).

        The call is sent at once and read with `async for`; leaving the loop,
        `aclose()`, `close()` or releasing the iterator cancels it.

        Raises:
            ClosedError: The Run is closed.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        warn_entry(KEYS, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return self._keys(prefix)


class KeyValueStore:
    """A Run's Key-value store, in the sync API. A program never constructs one."""

    def __init__(self, keys: Callable[[str | None], Sequence[str]]) -> None:
        """Takes what reads its keys under a prefix."""
        self._keys = keys

    def keys(self, *, prefix: str | None = None) -> Sequence[str]:
        """The committed keys that start with `prefix`, every key when left out.

        Sends the committed keys of a Run's Key-value store that start with
        `prefix`, in the order of their Unicode scalar values, in `$/key`
        notifications, one per credit granted with `$/credit`, then answers how many
        it sent. Takes no writer lock.

        Experimental: outside semver until it is promoted (ADR 0030).

        The call is sent at once and read with `for`; `close()`, leaving a
        `with` block or releasing the iterator cancels it.

        Raises:
            ClosedError: The Run is closed.
        """
        warn_entry(KEYS, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return self._keys(prefix)
