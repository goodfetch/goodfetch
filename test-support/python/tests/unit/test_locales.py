"""The shared table of locales and time zones (ADR 0073)."""

from datetime import date

from goodfetch_test_support import TimeZoneCase, locales, time_zones


def test_the_shared_locales_are_adr_0073s_table() -> None:
    """The locales are ADR 0073's seven tags, in the committed order."""
    tags = locales()

    assert tags == ["en-US", "de-DE", "fr-FR", "tr-TR", "ar-EG", "hi-IN", "ja-JP"]


def test_the_shared_time_zones_hold_the_dst_days_of_paris_and_new_york() -> None:
    """Paris and New York carry the 2026 days of their DST gap and overlap."""
    zones = time_zones()

    assert zones == [
        TimeZoneCase("UTC"),
        TimeZoneCase("America/St_Johns"),
        TimeZoneCase("Asia/Kathmandu"),
        TimeZoneCase("Pacific/Chatham"),
        TimeZoneCase("Australia/Lord_Howe"),
        TimeZoneCase("Europe/Paris", date(2026, 3, 29), date(2026, 10, 25)),
        TimeZoneCase("America/New_York", date(2026, 3, 8), date(2026, 11, 1)),
    ]
