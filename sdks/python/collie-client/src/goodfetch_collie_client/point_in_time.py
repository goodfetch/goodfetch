"""A point in time: a time-zone-aware `datetime` in UTC (ADR 0092).

`xtask codegen` gives every field the schema declares as a `date-time` this
type (spec 606, Casing and times). The engine writes RFC 3339 with any offset,
read as the same instant in UTC, and a `datetime` a program sends crosses the
wire as RFC 3339 in UTC. Hand-written, as the type's conversions are.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Annotated, TypeAlias

from pydantic import AfterValidator, AwareDatetime, PlainSerializer, TypeAdapter
from pydantic import ValidationError as _ValidationError

__all__ = ["PointInTime"]


def _in_utc(point: datetime) -> datetime:
    """The same instant, in UTC."""
    return point.astimezone(UTC)


def _rfc3339(point: datetime) -> str:
    """Its RFC 3339 text in UTC, which ends in `Z`."""
    return _in_utc(point).isoformat().replace("+00:00", "Z")


PointInTime: TypeAlias = Annotated[
    AwareDatetime,
    AfterValidator(_in_utc),
    PlainSerializer(_rfc3339, return_type=str, when_used="json"),
]
"""A point in time in UTC: built from any offset, sent as RFC 3339 in UTC."""

_POINT_IN_TIME: TypeAdapter[datetime] = TypeAdapter(PointInTime)


def point_in_time(sent: object) -> object:
    """The point in time the engine sent as RFC 3339 text, in UTC.

    A value that is no such text, such as one without an offset, stays as
    sent, as a result keeps whatever the SDK does not read (ADR 0082).
    """
    if not isinstance(sent, str):
        return sent
    try:
        return _POINT_IN_TIME.validate_strings(sent)
    except _ValidationError:
        return sent
