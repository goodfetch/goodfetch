"""A duration: a `timedelta` that crosses the wire as whole milliseconds (ADR 0080).

`xtask codegen` gives every field the schema declares as `Milliseconds` this
type. The engine writes a count of milliseconds, read as a `timedelta`, and a
`timedelta` a program sends crosses the wire as its whole milliseconds.
Hand-written, as the type's conversions are: Pydantic alone would read a
number as seconds and write ISO 8601.
"""

from __future__ import annotations

from datetime import timedelta
from typing import Annotated, TypeAlias

from pydantic import BeforeValidator, PlainSerializer

__all__ = ["Duration"]


def _of_milliseconds(sent: object) -> object:
    """The `timedelta` of a count of milliseconds; anything else as sent."""
    if isinstance(sent, int) and not isinstance(sent, bool):
        return timedelta(milliseconds=sent)
    return sent


def _milliseconds(duration: timedelta) -> int:
    """Its whole milliseconds, any part of one dropped."""
    return duration // timedelta(milliseconds=1)


Duration: TypeAlias = Annotated[
    timedelta,
    BeforeValidator(_of_milliseconds),
    PlainSerializer(_milliseconds, return_type=int, when_used="json"),
]
"""A duration: built from milliseconds, sent as whole milliseconds."""


def duration(sent: object) -> object:
    """The duration the engine sent as milliseconds.

    A value that is no count of milliseconds stays as sent, as a result keeps
    whatever the SDK does not read (ADR 0082).
    """
    return _of_milliseconds(sent)
