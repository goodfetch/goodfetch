"""JSON values, and the check that a value a program sends is one (ADR 0082, G3).

Only JSON crosses: a value that is not raises from the call that sent it,
naming its field path, rather than being turned into a string or dropped,
which would read back as a different value. User data keeps its keys as
given, `__proto__` included. TypeScript's `json.ts` is its twin.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import TypeAlias, cast

from pydantic import JsonValue

JSONValue: TypeAlias = JsonValue
"""A JSON value, as `json.loads` gives it: Pydantic's own, so a model's
field typed with it validates as JSON."""

JSONObject: TypeAlias = dict[str, JSONValue]
"""A JSON object."""


@dataclass(frozen=True)
class NonJSON:
    """Where a value that is not JSON sits, and what it is."""

    path: str
    """Its JSON Pointer (RFC 6901), empty at the root."""
    found: str
    """What it is: `NaN`, `an instance of Book`, `a cycle`, a key not a string."""


def non_json(value: object) -> NonJSON | None:
    """The first value in `value`, depth first, that is not JSON; `None` when all is.

    JSON is exactly a `dict` with `str` keys, a `list`, a `str`, an `int`, a
    finite `float`, a `bool` or `None`: a subclass, a tuple or a `Decimal`
    reads back as another type, so it is refused too.
    """
    return _first_non_json(value, "", set())


def pointer_token(key: str) -> str:
    """`key` as a JSON Pointer's reference token (RFC 6901)."""
    return key.replace("~", "~0").replace("/", "~1")


def _first_non_json(value: object, path: str, open_ids: set[int]) -> NonJSON | None:
    kind = type(value)
    if value is None or kind in (str, int, bool):
        return None
    if kind is float:
        return _non_finite(cast("float", value), path)
    if kind is not dict and kind is not list:
        return NonJSON(path, f"an instance of {kind.__qualname__}")
    if id(value) in open_ids:
        return NonJSON(path, "a cycle")
    open_ids.add(id(value))
    try:
        return _first_in_members(
            cast("dict[object, object] | list[object]", value), path, open_ids
        )
    finally:
        open_ids.discard(id(value))


def _non_finite(value: float, path: str) -> NonJSON | None:
    if math.isnan(value):
        return NonJSON(path, "NaN")
    if math.isinf(value):
        return NonJSON(path, "Infinity" if value > 0 else "-Infinity")
    return None


def _first_in_members(
    value: dict[object, object] | list[object], path: str, open_ids: set[int]
) -> NonJSON | None:
    members: list[tuple[str, object]]
    if isinstance(value, list):
        members = [(str(index), item) for index, item in enumerate(value)]
    else:
        members = []
        for key, item in value.items():
            if type(key) is not str:
                return NonJSON(path, f"the key {key!r}, not a string")
            members.append((key, item))
    for key, member in members:
        found = _first_non_json(member, f"{path}/{pointer_token(key)}", open_ids)
        if found is not None:
            return found
    return None
