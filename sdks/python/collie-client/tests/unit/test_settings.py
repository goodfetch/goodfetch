"""Settings in code (ADR 0083): a scope's settings as the wire takes them.

A duration goes in whole milliseconds, rounded up as a call's own timeout
is, and every other value as the program wrote it, `None` included, which
clears; the SDK checks no value the engine owns. TypeScript's
`settings.test.ts` is its twin.
"""

from __future__ import annotations

from datetime import timedelta

from goodfetch_collie_client.settings import settings_on_wire


def test_sends_a_duration_in_whole_milliseconds_rounded_up() -> None:
    """A `timedelta` goes in whole milliseconds, a part of one counting as one."""
    wire = settings_on_wire(
        {
            "timeout": {
                "navigation": timedelta(seconds=30),
                "action": timedelta(microseconds=1_500),
                "wait": timedelta(days=1),
            }
        }
    )

    assert wire == {"timeout": {"navigation": 30_000, "action": 2, "wait": 86_400_000}}


def test_sends_none_and_every_other_value_as_written_nested_by_segment() -> None:
    """`None` goes as null, which clears, and a tuple as a list."""
    wire = settings_on_wire(
        {
            "checkpoint": {"interval": None},
            "browser": {"block_resources": ("image", "font"), "headless": False},
            "tls": {"client_certificates": {"example.com": {"key": "env:KEY"}}},
            "run_state": {"batch": {"pages": "none"}},
        }
    )

    assert wire == {
        "checkpoint": {"interval": None},
        "browser": {"block_resources": ["image", "font"], "headless": False},
        "tls": {"client_certificates": {"example.com": {"key": "env:KEY"}}},
        "run_state": {"batch": {"pages": "none"}},
    }


def test_checks_no_value_the_engine_owns() -> None:
    """A zero duration and an unknown key go as written: the engine refuses them."""
    wire = settings_on_wire(
        {"timeout": {"navigation": timedelta(0)}, "proxy": {"unknown": -1}}
    )

    assert wire == {"timeout": {"navigation": 0}, "proxy": {"unknown": -1}}
