"""The shared table of locales and time zones (ADR 0073).

It is read from `test-support/fixtures/locales.json`, the one file Rust and
TypeScript read too, so Period Limits, Schedules and formatting take the same
zone cases in every language.
"""

from dataclasses import dataclass
from datetime import date
from typing import cast

from goodfetch_test_support.schemas import shared_fixture


@dataclass(frozen=True)
class TimeZoneCase:
    """One shared time zone, with its days of daylight saving time if it has them."""

    name: str
    """The zone's IANA name, such as `Europe/Paris`."""
    dst_gap: date | None = None
    """The day in 2026 whose local clock skips an hour (the gap)."""
    dst_overlap: date | None = None
    """The day in 2026 whose local clock repeats an hour (the overlap)."""


def _table() -> dict[str, object]:
    return cast(
        "dict[str, object]", shared_fixture("locales.json", "locales.schema.json")
    )


def locales() -> list[str]:
    """The shared locales' BCP 47 tags.

    Returns:
        `en-US`, `de-DE`, `fr-FR`, `tr-TR`, `ar-EG`, `hi-IN` and `ja-JP`.
    """
    return cast("list[str]", _table()["locales"])


def time_zones() -> list[TimeZoneCase]:
    """The shared time zones.

    Returns:
        `UTC`, `America/St_Johns`, `Asia/Kathmandu`, `Pacific/Chatham`,
        `Australia/Lord_Howe`, and `Europe/Paris` and `America/New_York`
        with their days of daylight saving time's gap and overlap.
    """
    zones = cast("list[dict[str, str]]", _table()["time_zones"])
    return [
        TimeZoneCase(
            zone["name"],
            _day(zone.get("dst_gap")),
            _day(zone.get("dst_overlap")),
        )
        for zone in zones
    ]


def _day(text: str | None) -> date | None:
    return None if text is None else date.fromisoformat(text)
